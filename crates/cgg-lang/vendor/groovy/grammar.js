/**
 * @file Apache Groovy grammar for tree-sitter
 * @author Elijah Zupancic <elijah@zupancic.name>
 * @license Apache-2.0 OR MIT
 *
 * The authoritative design document for this grammar is
 * SPECIFICATION.md at the repository root. Every rule below maps to
 * a section there. Read the spec before making non-trivial changes.
 *
 * The external scanner (src/scanner.c) handles the context-sensitive
 * lexing situations that pure grammar rules cannot disambiguate:
 *   - Slashy-string-start vs. division
 *   - Line / block / Groovydoc comment single-token emit
 *   - The contextual `record`, `trait` and `sealed` keywords, emitted
 *     only before a declaration so each word stays an identifier
 *     everywhere else, and `var`, `yield`, `in` and `as`, emitted only
 *     before what each declares, opens or takes
 *   - Decisions that depend on what shares a line: the key of a named
 *     argument in a parenthesis-free call, the start of such a call
 *     that is a value, a statement with a dotted receiver or a
 *     statement whose first argument is a `new`, an array type, a name
 *     or such a call in parentheses, a subscript `[`, an array type
 *     that is a value (`String[].class`), the `)` of a cast and the `)`
 *     before a relational operator Groovy applies to the parentheses
 *   - A `(` that opens a line after an expression and opens the next
 *     statement, and the `<` of a cast type's type arguments where they
 *     close
 *   - The `!in` operator, which depends on the character after it
 *   - The start of a lambda: the parameters in parentheses, or the one
 *     name, that `->` follows
 *   - The first comma of a multi-assignment, which it turns down where
 *     the parentheses go on as a method's parameters
 *   - The start of a typed local declaration (`String s`,
 *     `String s = 'a'`), which a line break after the name may end
 *   - The end of a lambda's brace body and of a block that opens a
 *     statement, at a `}` that no expression continues
 * Inside a string body it emits nothing. See SPECIFICATION.md §6 and
 * the `externals` list below.
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

/* eslint-disable no-multi-spaces --
 * The aligned trailing comments make the Apache-mirror precedence
 * ladder readable at a glance.
 */
const PREC = {
  // Higher number = tighter binding in tree-sitter.
  // Mirrors apache/groovy:GroovyParser.g4 `expression` rule.
  // See SPECIFICATION.md §3.1.
  LAMBDA: -1,                 // (a) -> body, x -> body: the body takes all it can
  ASSIGN: 1,                  // = += -= *= /= %= **= <<= >>= >>>= &= ^= |= ?=
  CONDITIONAL: 2,             // ?: (Elvis) and ? : (ternary) — same level
  IMPLICATION: 3,             // ==>
  LOGICAL_OR: 4,              // ||
  LOGICAL_AND: 5,             // &&
  BITWISE_OR: 6,              // |
  BITWISE_XOR: 7,             // ^
  BITWISE_AND: 8,             // &
  REGEX: 9,                   // =~ ==~                     (Apache level 8.5)
  EQUALITY: 10,               // == != <=> === !==
  RELATIONAL: 11,             // < <= > >= in !in
  // Apache level 7 is two alternatives, the first `as`, `instanceof` and
  // `!instanceof`, and in a left-recursive ANTLR rule an earlier
  // alternative binds tighter: `a < b instanceof T` is
  // `a < (b instanceof T)`.
  TYPE_TEST: 12,              // instanceof !instanceof as
  SHIFT_OR_RANGE: 13,         // << >> >>> .. ..< <.. <..<
  ADDITIVE: 14,               // + -
  MULTIPLICATIVE: 15,         // * / %
  UNARY: 16,                  // prefix ++ -- + -          (Apache level 3 UNARY_ADD)
  POWER: 17,                  // **                         (Apache level 2)
  UNARY_NOT: 18,              // ! ~                        (Apache level 1, tighter than POWER)
  // Apache `castExprAlt`, `castParExpression castOperandExpression`: the
  // operand is a postfix expression or a prefix operator before one, with
  // no binary operator, so `(int) a ** b` is `((int) a) ** b`.
  CAST: 18,                   // (T) x
  POSTFIX: 19,                // postfix ++ --, []
  ACCESS: 20,                 // . ?. ??. *. .& .@ ::
  PRIMARY: 21,                // literals, identifiers, ()
  // Type-context only — never competes with the expression ladder
  // above. Shares CONDITIONAL's numeric value by accident; the two
  // do not collide because `_type` and `_expression` reach the
  // resolver through disjoint parse stacks.
  ARRAY_TYPE: 2,
};
/* eslint-enable no-multi-spaces */

/*
 * Every weight below is a number of steps, which `weighted` turns into a
 * dynamic precedence of WEIGHT_UNIT per step. Tree-sitter adds up the
 * dynamic precedences in a reading, so a dynamic precedence of less than
 * one step can rank readings that the steps leave tied. It changes no order
 * that the steps set as long as its sum differs between two readings by
 * less than one step: for DOTTED_CAST_TYPE_WEIGHT, as long as they differ
 * in fewer than WEIGHT_UNIT dotted cast types.
 */
const WEIGHT_UNIT = 2;

/**
 * A rule with the dynamic precedence of a weight in steps.
 *
 * @param {number} steps The weight, in steps of WEIGHT_UNIT.
 * @param {RuleOrLiteral} rule The rule to weight.
 * @returns {PrecDynamicRule} The weighted rule.
 */
function weighted(steps, rule) {
  return prec.dynamic(steps * WEIGHT_UNIT, rule);
}

/*
 * Weights of class members that are tied to each other
 * (docs/divergences-from-spec.md §6). In a record, `static count`
 * followed by `R { }` is a modifier-only field and a compact
 * constructor, and also the typed field `static count R` followed by
 * an instance initializer. The first pair must outweigh the typed
 * field alone; equal sums would leave the choice to tree shape.
 */
const TYPED_FIELD_WEIGHT = 4;
const MODIFIER_ONLY_FIELD_WEIGHT = -1;
const COMPACT_CONSTRUCTOR_WEIGHT = TYPED_FIELD_WEIGHT - MODIFIER_ONLY_FIELD_WEIGHT + 1;

/*
 * The fork between a field's name and a type (the
 * `[variable_declarator, _type]` conflict) also splits a type nested
 * after a typed field with modifiers. In a record,
 * `private String y` followed by `class C extends B implements I { }`
 * is also the fields `String`, `y class`, `C extends` and
 * `B implements` and a compact constructor `I`: where a keyword has no
 * action it is an identifier. A type header splits into at most two
 * such fields after the one named by its keyword (`C extends`,
 * `B implements`), so a nested type in a record must outweigh a
 * modifier-only field, two typed fields and a compact constructor.
 */
const RECORD_TYPE_MEMBER_WEIGHT =
  MODIFIER_ONLY_FIELD_WEIGHT + 2 * TYPED_FIELD_WEIGHT + COMPACT_CONSTRUCTOR_WEIGHT + 1;

/*
 * The weight of a typed local variable declaration, which
 * its tokens also parse as an expression followed by an assignment.
 */
const TYPED_LOCAL_WEIGHT = 1;

/*
 * The weight of a cast whose operand is on the line of its
 * `)`, and the negative one of a cast whose operand is on a later line
 * (see `parenthesized_type_cast`).
 */
const CAST_WEIGHT = 1;

/*
 * The weight of a dotted type after `as`, `instanceof` or `!instanceof`,
 * alone or as the base of an array or generic type (`_dotted_type`). Its
 * first dot continues the type, as Groovy reads it (Apache's
 * `qualifiedClassName`), so the tokens no longer read as field accesses on
 * a cast of the first name; the weight ranks the cast against other readings
 * of the same lines: `x`, then `f() as a.B`, then `y = 1` is the cast and an
 * assignment, and also a bodiless method `x f()`, the name `as` and a typed
 * local `a.B y = 1`. It is half a step, so it ranks only readings that the
 * weights leave tied (see WEIGHT_UNIT). A whole step would tie readings that
 * a step now ranks: `(x) as` at the end of a line casts the name `as`, as
 * Groovy does, and `a.b` on the next line is a statement, which outweighs
 * `(x) as a.b` across the line break by CAST_WEIGHT.
 */
const DOTTED_CAST_TYPE_WEIGHT = 1 / WEIGHT_UNIT;

/*
 * The weight of the error production for a multi-assignment declaration
 * whose `=` does not come, the first of `_multi_assignment_valueless`; the
 * second, whose `=` comes, has none. Recovery by its missing value costs
 * what skipping a word of ten characters does, and
 * `def (a, b) abcdefghij = l` is the declaration with that word skipped
 * and also a declaration without a value before an assignment. Tied
 * recoveries are ranked by the order the parser reaches them in, which a
 * reparse changes, so the reparse and a parse from scratch chose
 * differently. Half a step down, the error production loses the tie in
 * both.
 */
const MISSING_VALUE_WEIGHT = -1 / WEIGHT_UNIT;

/*
 * The weight of a lambda. A closure parameter's default may
 * end where a lambda's parameters do: `def c = { a = (b) -> c }` is a
 * closure with the parameter `a` in Groovy, and also a closure whose body
 * assigns `a` the lambda `(b) -> c`. The weight must outweigh what else the
 * second reading may hold and the first not: a typed local and a cast, as
 * in `{ String a = (T) (b, c) -> d }`, where the first reading calls `(T)`.
 */
const LAMBDA_WEIGHT = -(TYPED_LOCAL_WEIGHT + CAST_WEIGHT + 1);

/*
 * The weight of a block where Groovy tries a block before a closure and
 * the braces end what they open: a `{` that opens a statement (Apache
 * `statement` lists `block` first), which `_statement_block_end` ends, and
 * the brace body of a lambda, which `_lambda_body_end` ends (Apache
 * `lambdaBody`). `{ a = b -> c }` that opens a statement is a block that
 * assigns a lambda, and also a closure with the parameter `a`, whose
 * reading holds no lambda: the block must outweigh the lambda in it. A
 * lambda whose block ends it weighs this and LAMBDA_WEIGHT, one step in
 * all, so each block of a nest, `if (x) { a = b -> { c = d -> e } }`,
 * outweighs the closure that would make its assignment a parameter, at any
 * depth. The weight is on every block rather than summed up at the end: the
 * parser keeps a few stack versions only, the ones of the highest weight so
 * far, and a reading that won only at its last `}` was dropped before it.
 */
const STATEMENT_BLOCK_WEIGHT = -LAMBDA_WEIGHT + 1;

/*
 * How many lambdas a lambda whose body is a closure (below) ranks under,
 * below the other readings of the same braces: a block that holds lambdas,
 * and a closure that starts the body (`a -> { b -> b } { c }`). It bounds
 * no nest of blocks that hold lambdas, as each such block outweighs a
 * closure (STATEMENT_BLOCK_WEIGHT).
 *
 * It must also outweigh the blocks that a reading gains over the braces
 * after such a lambda: where one reading takes `{ z }` on the lines after it
 * for closures that a call takes and the other for blocks that open
 * statements, each block adds STATEMENT_BLOCK_WEIGHT to the second.
 * `def c = { a = b -> { p -> p }` followed by lines of `{ z }` is the
 * parameter `a` in Groovy, and also a lambda whose body is a closure and a
 * block per line; at 10 eight lines made it the lambda, at 100 it takes 76.
 * The largest weight, BODY_BLOCK_WEIGHT, is then 604 in tree-sitter's
 * dynamic precedence, which a parse action holds in 16 bits.
 */
const LAMBDA_NESTING_DEPTH = 100;

/*
 * The weight of a lambda whose body is a closure, which another
 * reading of the same tokens holds otherwise. A lambda's body is a block
 * before a closure (Apache `lambdaBody`), so `x -> { a = b -> c }` must
 * rank its block, which holds a lambda, above the closure with the
 * parameter `a`. The weight is below that of LAMBDA_NESTING_DEPTH lambdas.
 */
const DEMOTED_LAMBDA_WEIGHT = LAMBDA_NESTING_DEPTH * LAMBDA_WEIGHT - 1;

/*
 * The weight of a lambda whose whole body is a parenthesized
 * expression, which is its body only before a `->` that the lambda cannot
 * take. `{ a = (b) -> (d) -> e }` is the parameter `a = (b)` and the body
 * `(d) -> e` in Groovy, which tries the parenthesized expression first, and
 * also the parameter `a = (b) -> (d)` and the body `e`: the weight is below a
 * lambda's. Where the parentheses before the `->` hold parameters and no
 * expression, Groovy's default does end at the parenthesized expression:
 * `{ T a = (b, c) -> (d) -> e }` is the parameter `a` with the default
 * `(b, c) -> (d)`, which outweighs the typed local `a` that holds two
 * lambdas only while it is just one below a lambda.
 */
const PARENTHESIZED_BODY_WEIGHT = LAMBDA_WEIGHT - 1;

/*
 * The weight of a block that is the body of `if`, `else`, a
 * loop or an arrow case, where Groovy tries a block before a closure
 * (Apache `statement` lists `block` first). `if (x) { a = b -> c }` is a
 * block that assigns a lambda, and also a closure with the parameter `a`,
 * whose reading holds no lambda. The block must outweigh what it holds and
 * the closure does not: a lambda whose body is a closure
 * (`if (x) { a = b -> { p -> p } }`), which is demoted.
 */
const BODY_BLOCK_WEIGHT = -DEMOTED_LAMBDA_WEIGHT + 1;

/*
 * The in-grammar `<` that opens type arguments, a diamond or type
 * parameters right after a name (§3.2.2). Where a comparison may follow
 * the same name, after `(i` or at a statement's start, both `<` tokens
 * match, and the lexer took this one, so `(i<n)` and `{ it<2 }` read
 * type arguments that never closed: an `ERROR`. The lower precedence makes
 * the comparison's `<` win there. Type arguments that close, or that run
 * past what the scanner reads, still open there with the scanner's
 * `_type_arguments_open`, which the lexer meets first; where only a type
 * may follow, this `<` is the only one.
 */
const TYPE_ARGUMENTS_LT = token.immediate(prec(-1, '<'));

/*
 * Identifier characters, after GroovyLexer.g4 `JavaLetter` and
 * `JavaLetterOrDigit`: Java's `Character.isJavaIdentifierStart` and
 * `isJavaIdentifierPart` without the identifier-ignorable characters.
 * In Unicode general categories that is L, Nl, Sc and Pc to start a
 * name, and Nd, Mn and Mc as well to continue it. `$` is in Sc and
 * `_` is in Pc.
 */
const ID_START = String.raw`\p{L}\p{Nl}\p{Sc}\p{Pc}`;
const ID_PART = ID_START + String.raw`\p{Nd}\p{Mn}\p{Mc}`;

/**
 * The source of an identifier pattern, in Rust regex syntax.
 *
 * The ASCII characters are spelled out and come first, so that the
 * lexer has a state that loops on them with plain comparisons. The
 * Unicode classes compile to a binary search over several hundred
 * ranges; written as one class, every character of every ASCII name
 * went through that search, which made the lexer about 2.5 times
 * slower per identifier character.
 *
 * @param {boolean} dollar Whether `$` is an identifier character. It
 *   is not in a GString `$name` path (GroovyLexer.g4
 *   `IdentifierInGString`).
 * @returns {string} The pattern source.
 */
function identifierPattern(dollar) {
  const asciiStart = dollar ? 'A-Za-z_$' : 'A-Za-z_';
  const asciiPart = asciiStart + '0-9';
  const otherStart = String.raw`[${ID_START}--[\x00-\x7F]]`;
  const otherPart = String.raw`[${ID_PART}--[\x00-\x7F]]`;
  const anyPart = `[${asciiPart}${otherPart}]`;
  return `(?:[${asciiStart}][${asciiPart}]*(?:${otherPart}${anyPart}*)?|${otherStart}${anyPart}*)`;
}

/**
 * GString body segment choice shared between `"..."` and `"""..."""`.
 * The text rule differs (single-quoted bans newlines, triple-quoted
 * allows them) but every other alternative is identical.
 *
 * `_string_body` is never emitted. Listing it here makes it valid
 * wherever a body segment may start, which is how the external scanner
 * learns that it is inside a string (SPECIFICATION.md §6.10).
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral} textRule The flavour-specific literal-text rule.
 * @returns {ChoiceRule} The shared body-segment choice.
 */
function gstringPart($, textRule) {
  return choice(
    alias(textRule, $.string_fragment),
    $.gstring_dollar_interpolation,
    $.gstring_brace_interpolation,
    alias($._gstring_literal_dollar, $.string_fragment),
    $._string_body,
  );
}

/**
 * The members of a type body, listed once. `enum_body` repeats the
 * list for the member directly after a trailing comma, where a plain
 * constructor is not allowed (see `enum_declaration`), and a record
 * body weights its nested types (`_record_type_member`); the lists
 * must not drift apart.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral | null} constructor The constructor rule for
 *   this position, or null where a constructor cannot appear.
 * @returns {RuleOrLiteral[]} The member rules, in a fixed order.
 */
function classMembers($, constructor) {
  return [...memberDeclarations($, constructor), ...typeDeclarations($)];
}

/**
 * The members of a type body other than nested types.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral | null} constructor See `classMembers`.
 * @returns {RuleOrLiteral[]} The member rules, in a fixed order.
 */
function memberDeclarations($, constructor) {
  return [
    $.method_declaration,
    alias($._default_method_declaration, $.method_declaration),
    $.field_declaration,
    alias($._multi_assignment_field, $.multi_assignment_declaration),
    ...(constructor ? [constructor] : []),
    $.static_initializer,
  ];
}

/**
 * The type declarations that may be nested in a type body.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {RuleOrLiteral[]} The declaration rules, in a fixed order.
 */
function typeDeclarations($) {
  return [
    $.enum_declaration,
    $.class_declaration,
    $.interface_declaration,
    $.trait_declaration,
    $.record_declaration,
    $.annotation_type_declaration,
  ];
}

/**
 * One or more modifiers, with annotations among them after the first:
 * Apache's `modifiers` and `classOrInterfaceModifiers` take annotations
 * and modifiers in any order: a field's `private` may come before its
 * annotation, as may a method's `public`. Annotations before the first
 * modifier stay in the declaration's own `repeat($.annotation)`, so each
 * annotation has one place to go. One hidden rule, `_modifiers`, holds the
 * list for every declaration, so that after a modifier the `@` of an
 * annotation and of `@interface` meet in one state (see `conflicts`).
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {SeqRule} The modifier list.
 */
function modifierList($) {
  return $._modifiers;
}

/**
 * The optional Groovy 4 `sealed` or `non-sealed` before `class`,
 * `interface` or `trait`, with the modifiers and annotations that may
 * follow it (`sealed abstract class W { }`, Groovy's own
 * `SealedSpecificationTest`): Apache's `classOrInterfaceModifiers` takes
 * them in any order. `sealed` comes from the scanner, which emits it
 * before one of those words, a modifier or an annotation on its line
 * (SPECIFICATION.md §6.13). `non-sealed` is a single token so that its
 * dash is not lexed as subtraction.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The optional modifier.
 */
function sealedModifier($) {
  return optional(seq(
    choice(alias($._sealed_keyword, 'sealed'), token('non-sealed')),
    repeat(choice($._modifier, $.annotation)),
  ));
}

/**
 * The `var` that stands where `def` or a type can, before a declared
 * name. The scanner emits it only when a name, or the `(` of a
 * multi-assignment after `final`, follows (SPECIFICATION.md §6.16), so
 * `var` is a name everywhere else (`int var = 1`, `var.foo()`,
 * `def f(var)`).
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {AliasRule} The keyword.
 */
function varKeyword($) {
  return alias($._var_keyword, 'var');
}

/**
 * A constructor. The enum position directly after a trailing comma
 * requires an access modifier (see `enum_declaration`); everywhere
 * else any modifiers are optional.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral} modifiers The modifier list for this position.
 * @returns {PrecRightRule} The constructor rule body.
 */
function constructorDeclaration($, modifiers) {
  return prec.right(3, seq(
    repeat($.annotation),
    modifiers,
    field('name', $.identifier),
    field('parameters', $.formal_parameters),
    optional($.throws_clause),
    field('body', $.block),
  ));
}

/**
 * A method declaration up to its parameter list: annotations,
 * modifiers, type parameters, `def` or a return type, the name and the
 * parameters. `method_declaration` and an annotation type element with
 * a default (`_annotation_type_element`) share it; a method with the
 * `default` modifier (`_default_method_declaration`) passes its own
 * modifiers.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral} [modifiers] What comes between the annotations
 *   and the name. By default, modifiers followed by type parameters and
 *   `def` or a return type, or modifiers alone.
 * @returns {RuleOrLiteral[]} The rules of the head, in order.
 */
function methodHead($, modifiers = choice(
  seq(
    optional(modifierList($)),
    optional(field('type_parameters', $.method_type_parameters)),
    choice('def', field('return_type', $._type)),
  ),
  modifierList($),
)) {
  return [
    repeat($.annotation),
    modifiers,
    field('name', $._property_name),
    field('parameters', $.formal_parameters),
  ];
}

/**
 * A method declaration from its head: the head and a throws clause,
 * then a body or nothing. See `method_declaration` for the weights.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral[]} head The rules of the head (`methodHead`).
 * @returns {PrecRightRule} The declaration rule body.
 */
function methodDeclaration($, head) {
  const signature = seq(...head, optional($.throws_clause));
  return prec.right(2, choice(
    weighted(-1, signature),
    weighted(3, seq(signature, field('body', $.block))),
  ));
}

/**
 * A method parameter or a closure parameter. Apache's `closure` takes
 * the `formalParameterList` of a method, so the two have one shape:
 * annotations, `final`, then `def`, `var` or a type (a `varargs_type`
 * included), the name, and a default value.
 *
 * Apache's `variableModifiersOpt` is wider: annotations and modifiers
 * in any order, `def` or `var` together with a type, and `...` with no
 * type. Those are not accepted (divergence 14).
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral} [value] The rule of a default value.
 * @returns {ChoiceRule} The parameter rule body.
 */
function parameter($, value = $._expression) {
  const defaultValue = optional(seq('=', field('default', value)));
  return choice(
    seq(
      repeat($.annotation),
      optional(seq('final', repeat($.annotation))),
      optional(choice('def', varKeyword($), field('type', choice($._type, $.varargs_type)))),
      field('name', $.identifier),
      defaultValue,
    ),
    // `_` is a literal directly after an annotation where a statement
    // can start (see `_placeholder_declarator`), and an annotated
    // closure parameter starts there too: `{ @A _ -> }`, or with `_`
    // as the type, `{ @A _ x -> }`. Anywhere else in a parameter `_`
    // is an ordinary identifier.
    seq(
      repeat1($.annotation),
      choice(
        field('name', alias('_', $.identifier)),
        seq(
          field('type', alias('_', $.type_identifier)),
          field('name', $.identifier),
        ),
      ),
      defaultValue,
    ),
  );
}

/**
 * The values of a `case`, a `value` field each, separated by commas.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {SeqRule} The value list rule.
 */
function caseValues($) {
  return seq(
    field('value', $._expression),
    repeat(seq(',', field('value', $._expression))),
  );
}

/**
 * One name of a multi-assignment declaration: a plain or typed name, a
 * rest binding or a keyed pair.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The name rule.
 */
function multiAssignmentName($) {
  return choice(
    $.variable_declarator,
    alias($._typed_multi_assignment_declarator, $.variable_declarator),
    alias($._rest_multi_assignment_declarator, $.rest_declarator),
    alias($._keyed_multi_assignment_declarator, $.variable_declarator),
  );
}

/**
 * The names of a multi-assignment declaration in their parentheses: the
 * list (`_multi_assignment_list`), then names that no input without an
 * error reaches, as the list takes every `,` after a name. They are what
 * error recovery goes on with when it goes back into a list that the
 * parser has reduced (`_multi_assignment_tail`).
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {RuleOrLiteral[]} The parentheses and what they hold, in order.
 */
function multiAssignmentNames($) {
  return [
    '(',
    $._multi_assignment_list,
    repeat(seq(',', multiAssignmentName($))),
    ')',
  ];
}

/**
 * The arguments of a parenthesis-free call after its first: after each
 * comma a named argument, any expression or a spread argument, Apache's
 * `argumentListElement` (`expressionListElement[true]` takes `*`).
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {RepeatRule} The repeated comma and argument.
 */
function commandArgumentsAfterFirst($) {
  return repeat(seq(',', field('argument', choice(
    alias($._command_named_argument, $.named_argument),
    alias($._command_continued_named_argument, $.named_argument),
    $._expression,
    $.spread_arguments,
    $.spread_map_entry,
  ))));
}

/**
 * The arguments of a parenthesis-free call that opens with a scanner
 * token (§6.14): the first a named argument or any expression, as in
 * Apache's `argumentList`, then the arguments after it.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {SeqRule} The argument list rule.
 */
function commandArguments($) {
  return seq(
    field('argument', choice(
      $._expression,
      alias($._command_named_argument, $.named_argument),
    )),
    commandArgumentsAfterFirst($),
  );
}

/**
 * The member accesses that may join the names of a parenthesis-free
 * call's receiver (§5.5 item 2): the node each one is in an expression,
 * its operator, and the field of the name after the operator.
 */
const COMMAND_PATH_ACCESSES = [
  ['field_access', '.', 'field'],
  ['safe_navigation_expression', '?.', 'property'],
  ['safe_chain_dot_expression', '??.', 'property'],
  ['spread_dot_expression', '*.', 'property'],
  ['method_pointer_expression', '.&', 'method'],
  ['direct_field_access_expression', '.@', 'field'],
  ['safe_direct_field_access_expression', '?.@', 'field'],
  ['safe_chain_direct_field_access_expression', '??.@', 'field'],
  ['spread_direct_field_access_expression', '*.@', 'field'],
];

/**
 * The rules of a receiver that is a name followed by member accesses: a
 * hidden rule `path` choosing among one hidden rule per access, each
 * aliased to the expression node of its operator, so the receiver has
 * the nodes and fields of the same text anywhere else. An access joins
 * the receiver's first name, as `first` builds it, or another access.
 *
 * @param {string} path The name of the hidden choice rule.
 * @param {(function(GrammarSymbols<string>): RuleOrLiteral)} first The
 *   object of the innermost access.
 * @returns {{[name: string]: RuleBuilder<string>}} The rules, by name.
 */
function commandPathRules(path, first) {
  const rules = {
    [path]: $ => choice(...COMMAND_PATH_ACCESSES.map(([kind]) =>
      alias($[`${path}_${kind}`], $[kind]))),
  };
  for (const [kind, operator, nameField] of COMMAND_PATH_ACCESSES) {
    rules[`${path}_${kind}`] = $ => seq(
      choice(first($), field('object', $[path])),
      operator,
      field(nameField, reserved('property', $.identifier)),
    );
  }
  return rules;
}

/**
 * An expression or a parenthesis-free call, Apache's
 * `enhancedStatementExpression` without lambdas: the value of a
 * declarator (`variableInitializer`) or of an assignment (the right side
 * of `#assignmentExprAlt` and `#multipleAssignmentExprAlt`), what is in
 * parentheses (`expressionInPar`, also around the condition of `if`,
 * `while` and `do`, a `switch` value and a `synchronized` lock), and a
 * `${…}` interpolation, whose closure body holds the call as its one
 * statement.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The value rule.
 */
function statementExpression($) {
  return choice($._expression, alias($._value_command_chain, $.command_chain));
}

/**
 * A `statementExpression` or a lambda whose one parameter has no
 * parentheses (`x -> x + 1`), Apache's `enhancedStatementExpression`: the
 * value of a declarator or an assignment other than `(a, b) = …`, and what
 * is in parentheses. Apache takes such a lambda nowhere else but in an
 * argument and the value of a map entry or a named argument
 * (`enhancedArgumentListElement`, `enhancedExpression`), so
 * `return x -> x`, `[x -> x]` and `a -> b -> c` stay errors. A lambda with
 * parentheses is an `_expression`.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The value rule.
 */
function enhancedStatementExpression($) {
  return choice(statementExpression($), bareLambda($));
}

/**
 * A lambda whose one parameter has no parentheses, aliased to
 * `lambda_expression`.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {AliasRule} The lambda rule.
 */
function bareLambda($) {
  return alias($._bare_lambda_expression, $.lambda_expression);
}

/**
 * An expression, or a lambda whose one parameter has no parentheses:
 * Apache's `enhancedExpression`, the value of a map entry or a named
 * argument (Groovy 5 and later), and an argument in parentheses.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The value rule.
 */
function enhancedExpression($) {
  return choice($._expression, bareLambda($));
}

/**
 * The body of a lambda, Apache's `lambdaBody`: a block, or an expression
 * or a parenthesis-free call. A `{` opens a block wherever one parses, as
 * Apache tries the block first: `() -> { }` and `x -> { a = b -> c }` hold
 * blocks, `x -> { y -> y }` a closure. A closure and a parenthesized
 * expression are also alternatives of their own, ranked low
 * (DEMOTED_LAMBDA_WEIGHT, PARENTHESIZED_BODY_WEIGHT); the static precedence
 * makes the parser reduce the parenthesized expression here rather than as
 * an expression, which would carry no weight. A lambda without parentheses
 * is no body, so `a -> b -> c` is an error in Groovy and here;
 * `a -> (b) -> c` is not.
 *
 * A brace body ends at its `}` only where `_lambda_body_end` (§6.19)
 * follows, which the scanner emits unless an expression continues there.
 * Otherwise the braces are a closure that starts the body:
 * `a -> { y -> y }.call()` and `a -> { } + 1` call and add the closure, as
 * in Groovy. A lambda in parentheses is an expression that may go on
 * itself, so its block may be followed by anything (`(a) -> { foo() }.call()`
 * calls the lambda); where the braces are a block and a closure alike, the
 * block's static precedence keeps the block. A lambda without parentheses
 * may not, so its block needs the token.
 *
 * Each alternative carries the whole weight of the lambda, as a production
 * takes the largest of the weights in it, not their sum: LAMBDA_WEIGHT, and
 * for a block that the token ends STATEMENT_BLOCK_WEIGHT as well.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {boolean} bare Whether the lambda has no parentheses.
 * @returns {ChoiceRule} The body rule, its `body` field on the body alone.
 */
function lambdaBody($, bare) {
  const body = (rule) => field('body', rule);
  return choice(
    weighted(
      LAMBDA_WEIGHT + STATEMENT_BLOCK_WEIGHT,
      seq(body($.block), $._lambda_body_end),
    ),
    ...(bare ? [] : [weighted(LAMBDA_WEIGHT, body($.block))]),
    weighted(DEMOTED_LAMBDA_WEIGHT, seq(body($.closure), $._lambda_body_end)),
    weighted(PARENTHESIZED_BODY_WEIGHT, prec(1, body(alias(
      $._lambda_parenthesized_expression,
      $.parenthesized_expression,
    )))),
    weighted(LAMBDA_WEIGHT, body(statementExpression($))),
  );
}

/**
 * A block that opens a statement, as Apache's `statement` takes it first:
 * at the start of a statement in a list of them and after a label. A body
 * of `if`, `else`, a loop or an arrow case takes its block directly
 * (BODY_BLOCK_WEIGHT). `_statement_block_end` (§6.22) ends it, which the
 * scanner emits after the `}` unless an expression continues the braces
 * there, as it decides `_lambda_body_end` where a statement may follow; where
 * one does they are a closure: `{ foo() }()`, `{ foo() }.call()`,
 * `{ foo() } as Runnable` and `{ foo() }` followed by `.call()` on the next
 * line call and cast it, as in Groovy. A token of its own rather than
 * `_lambda_body_end` keeps the states after the `}` of a statement's block
 * and of a lambda's apart. One state for both made
 * `def c = { a = a -> { foo() }` followed by `+ 1` on the next line a lambda
 * with a block where Groovy reads the parameter `a` and a block: the stack
 * version split off at the block's reduction lexes again in the state after
 * the block, where the scan, as after a reused block, hands back every `}`
 * that a line with an operator follows, and so ends.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {(rule: RuleOrLiteral) => RuleOrLiteral} [wrap] Wraps the block,
 *   to put a field on it and not on the token.
 * @returns {PrecDynamicRule} The block rule.
 */
function statementBlock($, wrap = (rule) => rule) {
  return weighted(STATEMENT_BLOCK_WEIGHT, seq(wrap($.block), $._statement_block_end));
}

/**
 * A list of statements and `;`, in a file, a block, a closure and a colon
 * case: a statement may be a block there.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {RepeatRule} The list rule.
 */
function statementList($) {
  return repeat(choice($._statement, statementBlock($), ';'));
}

/**
 * The braces of an array initializer around `element`s separated by
 * commas, with a trailing comma allowed after the last one (Apache
 * `variableInitializers`).
 *
 * @param {RuleOrLiteral} element An element of the initializer.
 * @returns {SeqRule} The initializer rule body.
 */
function arrayInitializer(element) {
  return seq(
    '{',
    optional(seq(element, repeat(seq(',', element)), optional(','))),
    '}',
  );
}

/**
 * The `(` of a rule that may open a statement: a parenthesized expression,
 * a cast or a multi-assignment without `def`. Besides the in-grammar `(` it
 * takes `_statement_paren` (§6.18), the `(` the scanner emits where a line
 * starts with one after an expression and a statement may start. A call's
 * arguments take only the in-grammar `(`, so such a line opens the next
 * statement, as in Groovy: `foo()` followed by `(a, b) = [1, 2]` is a call
 * and a multi-assignment.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @returns {ChoiceRule} The opening parenthesis.
 */
function statementParen($) {
  return choice('(', alias($._statement_paren, '('));
}

/**
 * The parentheses of a call or an annotation around `argument`s separated
 * by commas, with a trailing comma allowed after the last one.
 *
 * @param {RuleOrLiteral} argument An argument of the list.
 * @returns {SeqRule} The argument list rule body.
 */
function argumentList(argument) {
  return seq(
    '(',
    optional(seq(argument, repeat(seq(',', argument)), optional(','))),
    ')',
  );
}

module.exports = grammar({
  name: 'groovy',

  // Keywords are extracted from this token. A keyword is lexed as
  // such only in a state where it has a parse action, or where it is
  // reserved (`reserved` below); anywhere else the same text is an
  // `identifier`. Several comments in this file depend on that.
  word: $ => $.identifier,

  // A rule listed here adds no node of its own: its body is copied into
  // each rule that uses it. `[(Foo)` then `(y)(z)]` has two readings of
  // equal weight, the cast of the cast `(y)(z)` (the later-line cast's
  // -CAST_WEIGHT and the same-line cast's +CAST_WEIGHT cancel) and the
  // call `(Foo)(y)(z)`. With the element directly in the list the cast
  // wins, as at v0.6.0 and as in Groovy; a hidden `_list_element` node
  // made the runtime keep the call. Inlined, it costs 3 parse states.
  inline: $ => [$._list_element],

  // §5.2 — `new` is a hard keyword. Keyword extraction alone still
  // lexes it as `identifier` wherever the keyword has no action,
  // which let `def f = (Foo) new T()` after another member split
  // into a field `= (Foo)` and a method `T()` returning `new`. The
  // empty `property` set keeps `foo.new` (Apache `namePart`) valid;
  // `_property_name` also names methods, so `new` is not reserved
  // where a method name may follow (`def new`, `int new`).
  //
  // `case` is reserved as in Groovy. While it was a name, error recovery
  // on `case 1:` lines outside a switch kept a version that nested each
  // line as a label in the one before and closed the whole nest at every
  // line: time quadratic in the lines. It stays a key where Groovy's
  // `namedArgLabel`, `mapEntryLabel` and `elementValuePairName` take
  // keywords: an alternative of `map_entry`, of the named argument after
  // a comma and of an annotation's `name = value`, and the empty
  // `property` set for a key after `_command_named_key` and a name after
  // a member access (`x.case`, `x::case`).
  reserved: {
    global: _ => ['new', 'case'],
    property: _ => [],
  },

  extras: $ => [
    /\s/,
    // §5.10 — a line escape, `\` at the end of a line. Groovy's lexer
    // skips it as it skips spaces and tabs.
    /\\\r?\n/,
    $.line_comment,
    $.block_comment,
    $.groovydoc_comment,
    // §6.6 — an empty, hidden token at the first `/*` that has no `*/`.
    // It adds no syntax: the scanner needs a token to carry that fact
    // forward, and only an extra is valid wherever a comment may start.
    // §6.19 has the scanner emit it after the `}` of a lambda's brace body
    // too, to carry a decision made past the token that follows.
    $._unterminated_comment,
  ],

  // Externals listed here must have a matching branch in
  // `src/scanner.c` AND be referenced by at least one rule below
  // (`_command_named_key` and `_subscript_open` included) or by
  // `extras`. There are three exceptions. No rule references
  // `_error_sentinel`: it exists so the scanner can detect error
  // recovery, when tree-sitter marks every external valid. It is kept
  // last here and in the scanner's `TokenType` enum, and the two
  // orders must match. No rule references `_lambda_body_reread` either:
  // the scanner emits it where no rule takes it (§6.19, §6.22). The scanner
  // never emits `_string_body`, `_type_start`, `_closure_parameters_start`,
  // `_type_only_close` or `_body_block_end`, nor `_error_sentinel`: a string
  // body, a type, closure parameters, the cast of an array or generic type
  // whose operand is on a later line and the block that is the body of `if`,
  // `else`, a loop or an arrow case reference the first five so that the
  // scanner can tell it is inside one or where one may start or end.
  // New tokens go last but for the sentinel, so that they renumber no
  // symbol before them: `_statement_paren`, first listed after
  // `_subscript_open`, renumbered the symbols after it there, which changed
  // error recovery (`enum E<T> { A }` lost its `enum_declaration`).
  // Tokens the spec §6 proposes as external scanner branches but
  // that this grammar implements in-grammar (GString interpolation
  // via regex tokens, slashy body via in-grammar `_slashy_text`,
  // label colon via grammar precedence, and statement boundaries
  // via `statementList` — newline is `extras` whitespace and `;` is an
  // anonymous list element, see §5.10)
  // are intentionally NOT listed: listing an unwired external token
  // would let dormant scanner branches mis-fire in error-recovery
  // states.
  externals: $ => [
    $._slashy_string_start,
    $.line_comment,
    $.block_comment,
    $.groovydoc_comment,
    $._record_keyword,
    // §6.13 — `trait` before a trait header, `sealed` before `class`,
    // `interface` or `trait`. Anywhere else each word is an identifier.
    $._trait_keyword,
    $._sealed_keyword,
    // §6.16 — `var`, `yield`, `in` and `as`, each emitted only where its
    // construct follows: `var` before a declared name, `var` before the
    // `(…) =` of a multi-assignment, `yield` and `in` before an
    // expression, `as` before a type. Anywhere else each word is an
    // identifier.
    $._var_keyword,
    $._var_names_keyword,
    $._yield_keyword,
    $._in_keyword,
    $._as_keyword,
    // §5.5 — empty token before the `name:` key of a named argument in
    // a parenthesis-free call. Scanner-side so the newline before a Spock
    // label (`result` then `where:`) is not swallowed, and so
    // `Type name =` stays a declaration.
    $._command_named_key,
    // §6.16 — the key `var`, `yield`, `in` or `as` of such an argument,
    // the word itself.
    $._contextual_key,
    // §6.14 — empty token before the receiver of a parenthesis-free call
    // that is a value (`def x = foo 1`), emitted only when the first
    // argument starts on the receiver's line. `def x = foo` followed by
    // `1` on the next line stays two statements.
    $._command_value_start,
    // §6.14 — empty token between the first name of a statement and a
    // member access after it (`obj` and `.method 1`), emitted only when
    // the first argument follows on the line.
    $._command_path_start,
    // §6.14 — empty token before the first name of a statement that is a
    // parenthesis-free call whose first argument the grammar cannot tell
    // from a following statement (`foo new Bar()`, `foo String[].class`),
    // emitted only when that argument starts on the receiver's line.
    $._command_statement_start,
    // §5.10 — `[` on the same line as the token before it (spaces, tabs and
    // line escapes allowed). A `[` after a newline isn't one and can only
    // start a list or map literal: a statement, not a subscript of the line
    // above.
    $._subscript_open,
    // §6.15 — empty token before an array type that is a value
    // (`String[].class`, `foo(int[])`), emitted when a name and `[]`
    // follow. Without it the parser would learn that `String` is a type
    // only at `]`, after it had reduced the name to an expression.
    $._array_value_start,
    // §6.11 — a `)` with something after it on the same line. A cast
    // that closes with it outranks a parenthesized expression; a cast
    // whose operand is on a later line ranks below one, so `x = (a)`
    // followed by `foo()` is two statements.
    $._cast_close,
    // §6.12 — the `!in` operator, which needs whitespace or an opening
    // bracket after it. The scanner reads that character and leaves it.
    $._not_in,
    // §6.17 — empty token before the `(` of a lambda's parameters, emitted
    // when what the parentheses hold reads as parameters and `->` follows
    // them. Without it the parser would learn that `(a)` opens a lambda only
    // at the `->`, after it had reduced `a` as an expression, a cast's type
    // and a parameter.
    $._lambda_start,
    // §6.17 — empty token before the name of a lambda's one parameter
    // without parentheses (`x -> x`), emitted when `->` follows the name.
    $._bare_lambda_start,
    // §6.6 — see `extras`.
    $._unterminated_comment,
    // §6.10 — never emitted: valid only between the quotes of a string
    // whose body is lexed in pieces. Comments are extras, so the parser
    // offers them there too; the scanner must emit nothing, or `//` and
    // `/*` at a piece boundary become comments.
    $._string_body,
    // §6.15 — never emitted: valid wherever a type may start, so that the
    // scanner can tell `(String[]) x` from `foo(String[])`.
    $._type_start,
    // §6.18 — a `(` that starts a line after an expression, where a
    // statement may start and the parentheses can open one. Groovy ends
    // the statement at that line break, so the `(` opens the next
    // statement rather than a call's arguments of the line above.
    $._statement_paren,
    // §6.19 — empty token after the `}` of a lambda's brace body that is
    // its whole body: emitted unless what follows continues an expression.
    $._lambda_body_end,
    // §6.19 — empty, and like `_error_sentinel` taken by no rule: emitted
    // after a lambda's block that the parser shifted whole where an
    // expression continues the braces, so that the runtime breaks a reused
    // block down and the `}` is decided where a parse from scratch decides
    // it.
    $._lambda_body_reread,
    // §6.20 — the first `,` of `(a, b) = …`, emitted unless the parentheses
    // go on as a method's parameters (`def f(a, b) { }`). Without it the
    // reading in which the `(` after `def f` opens the next statement held
    // `def f(a, b)` to the token after the `)`.
    $._multi_assignment_comma,
    // §6.21 — empty token before the type of a typed local declaration
    // (`String s`, `int a, b`, `String s = 'a'`), emitted when the
    // declaration ends on the name's line, an initializer or a list of names
    // follows.
    $._declaration_start,
    // §6.22 — empty token after the `}` of a block that opens a statement,
    // emitted unless what follows continues an expression, so that the
    // braces are a closure there (`{ foo() }()`).
    $._statement_block_end,
    // §6.11 — a `)` before a relational operator that Groovy applies to the
    // parentheses (`(x) instanceof Foo`, `(x) !in y`, `(x) as Foo`), which
    // closes no cast. The scanner also emits it where only a cast's type
    // takes the `)`, where no rule takes it, so that the cast reading ends
    // there.
    $._relational_close,
    // §3.2.2 — the `<` of a cast type's type arguments, emitted only where
    // they close (`x as List<String>`), so that `x as a.B<c` compares the
    // cast with `c`.
    $._cast_type_arguments_open,
    // §6.21 — never emitted: valid where closure parameters may start, so
    // that the scanner can tell `{ String a = b -> c }`, whose `->` may end
    // a parameter list, from a typed local's lambda value elsewhere.
    $._closure_parameters_start,
    // §6.11 — never emitted: valid before the `)` of a cast whose operand
    // is on a later line and whose type is an array or generic type, which
    // no expression is, so that the scanner keeps that cast.
    $._type_only_close,
    // §6.17 — never emitted: valid after the `}` of the block that is the
    // body of `if`, `else`, a loop or an arrow case, where a `(` opens the
    // next statement on any line, so that the scanner decides a lambda there
    // although the braces may be a closure too.
    $._body_block_end,
    // §6.23 — empty token after `return`, `break` or `continue` where a
    // line break or the end of input comes before the next token, which
    // ends the statement: Groovy takes the value or the label only on the
    // keyword's line.
    $._jump_statement_end,
    // §6.24 — empty tokens before the first name of a statement that calls
    // it with a closure that closes on the name's line and that a member
    // access follows (`foo { }.bar()`); the second where the accesses are
    // the receiver of a parenthesis-free call (`foo { }.bar 1`).
    $._closure_call_start,
    $._closure_command_start,
    // §5.2 — never emitted: ends `_stray_case`, the error production for a
    // `case` label outside a switch, so that the label is never accepted.
    $._unreachable,
    // §3.2.2 — the `<` of a generic type's arguments where they close or
    // run past the scanner's read, so that `(i<n)` compares and
    // `(List<String>) x` casts.
    $._type_arguments_open,
    // §6.11 — a `)` before a name on its line, emitted in place of
    // `_cast_close` where it is valid. The name is the operand of a cast in
    // Groovy, and could also start a member that would split the line
    // (see `parenthesized_expression`).
    $._cast_word_close,
    // Never referenced by a rule: valid only during error recovery, when
    // the scanner must not emit context-sensitive tokens.
    $._error_sentinel,
  ],

  conflicts: $ => [
    // After a modifier an `@` opens an annotation among the modifiers or
    // `@interface` (`public @Deprecated @interface X { }`): the word after
    // it decides, one token past what the table sees.
    [$._modifiers],
    [$._type_modifiers],
    // An annotation among a field's modifiers before the parentheses of a
    // multi-assignment (`private @Lazy (int x, String y) = load()`): the
    // `(` opens its arguments or the names, and the `=` after them decides.
    [$.annotation],
    // `final def x` / `final x = 1` in a block vs. a modified member at top level.
    [$._modifier, $.local_variable_declaration],
    // `final def (a, b) = …` takes the same fork at `final`: the
    // multi-assignment shifts `def` with the local, so the fork is no
    // wider. The second entry is the same fork at the start of a closure
    // (see below).
    [$._modifier, $.local_variable_declaration, $.multi_assignment_declaration],
    // This array is the authoritative list; SPECIFICATION.md §7 is
    // the list predicted before the grammar existed.
    // (list_literal vs map_literal — tree-sitter resolves without
    //  an explicit conflict because `:` after the first element
    //  always disambiguates within one token of lookahead.)

    // §5.15 — inside `( … )` an identifier could be either an
    // _expression (parenthesized_expression) or a _type
    // (parenthesized_type_cast). Tree-sitter explores both via
    // this conflict; the `)` plus the next token disambiguates.
    [$._expression, $._type],
    // The same for the type of a cast whose operand is on a later line.
    [$._expression, $._cast_name_type],

    // §4 — `a.b.c` could be a field_access chain (expression
    // context) or a qualified_type (type context). Same family
    // as the _expression / _type conflict above but the implicit
    // qualified_type_repeat1 aux symbol forces a separate
    // declaration.
    [$._expression, $.qualified_type],

    // §3.2.2 — at a `[` after the type of `x as T`, the type is a cast's
    // whole type, which a subscript follows (`x as T[0]`), or the element
    // of an array type (`x as T[]`); the `]` after the `[` decides.
    [$._cast_type, $._cast_array_element],

    // §4 — `for ( identifier …` could continue as a for_statement
    // (identifier is part of the init expression) or as a
    // for_in_statement (identifier is the loop variable). The
    // token after the identifier (`;` / operator vs `in`)
    // disambiguates.
    [$.for_in_statement, $._expression],

    // §5.2 — `def identifier` could either be a
    // local_variable_declaration (variable_declarator with no
    // initializer) or the start of a method_declaration
    // (_property_name + parameters). The `(` after the name
    // decides; the conflict keeps both alternatives alive until
    // then.
    [$.variable_declarator, $._property_name],

    // §4 — `static NAME` inside a class body: after one or more
    // modifiers and one identifier, that identifier could be the
    // modifier-only field_declaration's variable_declarator name
    // (`static MEDIUM_TTL = 900`) or the start of a typed
    // field_declaration/method_declaration's `_type` (`static Foo
    // bar`). The next token (`=`/`,`/`;` vs a second identifier)
    // disambiguates.
    [$.variable_declarator, $._type],

    // §4 — `enum E { E(v, w) { } }`: after `E(` an identifier could be
    // a constant's argument (`_expression`) or a constructor's untyped
    // parameter (`formal_parameter`); the `{` after `)` decides. The
    // conflict is global by necessity — it is between the two
    // reductions, not their containers. Outside an enum body it is live
    // in one more place: after `_lambda_start` (§6.17), where `(b)` opens
    // a lambda's parameters or, as a closure parameter's default or a case
    // value, is an expression (`_lambda_parenthesized_expression`).
    [$.formal_parameter, $._expression],
    // §4 — after an enum's trailing comma, `private` may open a
    // constructor (`private E() { }`) or any other member.
    [$._modifier, $._modified_constructor_declaration],
    // §5.8 — a closure's parameter list and a closure body that opens
    // with a declaration share their first tokens, and so does a
    // block, whose `{` may also open a closure. The `->` decides, or
    // the first token that only one reading accepts.
    //
    // `{ final x -> }` vs. a body opening with a final local (#56) or
    // with a modified declaration; all three stay alive at `final`.
    [$._modifier, $.local_variable_declaration, $.closure_parameter],
    // `{ final @A x -> }`: an annotation after `final` in a closure's first
    // parameter, as in a method's (Apache `variableModifiers`), against
    // `final` as the modifier of a member with annotations after it.
    [$._modifier, $.closure_parameter],
    // `{ final def (a, b) = … }`: the same fork, no wider.
    [$._modifier, $.local_variable_declaration, $.multi_assignment_declaration, $.closure_parameter],
    // `{ String a = 'x', b -> }` vs. `{ String a = 'x', b = 'y' }`, and
    // `{ def a, b -> }` vs. `{ def a, b }`. These two fork at every
    // `,` of a declaration that opens a closure or block, and both
    // stacks run to the end of the list: 15% to 60% more work for a
    // list of two to eight names, about twice for a long one. A list
    // of one name does not fork.
    [$._initialized_declarator, $.closure_parameter],
    [$.variable_declarator, $.closure_parameter],
    // `{ @A x = 1, y -> }` against `{ @A x = 1, y }`, the annotated
    // declaration with no type; the `->` decides.
    [$._valued_declarator, $.closure_parameter],
    // `{ final String a, b -> }` vs. `{ final String a, b }`, the same fork
    // after a blank final's name.
    [$.local_variable_declaration, $.closure_parameter],
    // `{ @A _ x -> }`, a parameter of type `_`, vs. `{ @A _` as the
    // declaration of the placeholder followed by a statement `x`.
    [$._placeholder_declarator, $.closure_parameter],
    // §3.2 — `new T[][] { {a, b} }` vs. `new T[][] { {a, b -> a} }`: in
    // a nesting array initializer a `{` opens an initializer or a closure,
    // and a name before a `,` is an element or a parameter. The `->`
    // decides; an initializer holds none.
    [$._expression, $.closure_parameter],
    // §5.16 — `{ a = b -> c }`: the closure parameter `a = b`, or a closure
    // or a block whose statement assigns `a` a lambda. See `_bare_lambda_name`.
    [$._bare_lambda_name, $._bare_lambda_expression],
    // An untyped default, `{ a = 1 -> }` vs. `{ a = 1 }`, needs no
    // entry: see `expression_statement`.
    //
    // Two things keep these forks from changing a valid parse, and a
    // new rule must keep both. `closure_parameter` carries no dynamic
    // precedence, so a reading that survives to the `->` wins by being
    // the only one left. And `->` appears at the top level of a closure
    // body only where a reading without it is ranked first: in the
    // arrow forms of `switch`, and in a lambda, whose weight is negative
    // (LAMBDA_WEIGHT) but for the block that ends it, which weighs as much
    // as the block the parameter's reading takes the braces for instead
    // (STATEMENT_BLOCK_WEIGHT). `{ a = (b) -> c }` holds both: the
    // parameter `a`, and `a` assigned the lambda `(b) -> c`; Groovy reads
    // the parameter, and in `{ a = (b) -> { } }` the parameter and a block.
    // On invalid input the parameter reading can win, because a missing
    // `->` is a cheaper repair than an ERROR: `{ x = 1, y }` recovers as
    // two parameters.
  ],

  rules: {
    // Top-level. Real rule set lands incrementally per
    // SPECIFICATION.md §4 (statement and declaration coverage).
    source_file: $ => seq(
      optional($.shebang),
      statementList($),
    ),

    // §4 — `#!/usr/bin/env groovy` line at file start. Single
    // token consumed by the in-grammar lexer.
    shebang: _ => token(seq('#!', /[^\n]*/)),

    // §4 — statement-level alternatives. Declarations and the rest
    // of control flow fill out this choice in subsequent iterations.
    // A `{` that opens a statement is a block (`statementBlock`), which
    // the lists of statements and a label take besides this choice: a
    // body of `if`, `else`, a loop or an arrow case takes its block
    // directly, and through this choice it would reach a second block
    // that the scanner decides after every such body's `}`, which cost
    // 13% more instructions on code made of conditions.
    _statement: $ => choice(
      $.expression_statement,
      $.if_statement,
      $.while_statement,
      $.do_while_statement,
      $.for_statement,
      $.for_in_statement,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.throw_statement,
      $.assert_statement,
      $.yield_statement,
      $.try_statement,
      $.synchronized_statement,
      $.class_declaration,
      $.trait_declaration,
      $.interface_declaration,
      $.annotation_type_declaration,
      $.enum_declaration,
      $.record_declaration,
      $.package_declaration,
      $.import_declaration,
      $.local_variable_declaration,
      $.multi_assignment_declaration,
      $.pipeline_statement,
      $.method_declaration,
      alias($._default_method_declaration, $.method_declaration),
      $.labeled_statement,
      $.command_chain,
      alias($._path_command_chain, $.command_chain),
      alias($._stray_case, $.expression_statement),
    ),

    // §5.2 — a `case` label where a statement starts and no switch body
    // goes on: an error production. `$._unreachable` never comes, so the
    // label is always an error, and recovery inserts it as a missing
    // `switch` after the `:`. Without this rule the reserved `case` cannot
    // be shifted there, although every statement before it is reduced on
    // it (inside a switch it ends a case). Recovery then skipped it, read
    // `A:` of `case A:` as a label and the next line as that label's
    // statement, and at each further `case` a version closed the whole
    // nest of labels before it failed: time quadratic in the lines. In a
    // switch body a `case` after a case's statements still ends that case
    // (`switch_case` is left-associative), so no valid tree changes.
    _stray_case: $ => seq(
      'case',
      $._expression,
      ':',
      alias($._unreachable, 'switch'),
    ),

    // §5.5 — command_chain is the parenthesis-free method-call
    // form (`println 'hello'`, `events 'a', 'b', 'c'`). This rule is the
    // statement whose receiver is one name and whose first argument is
    // a literal that is the whole argument, a closure or a named argument,
    // which no other statement starts with, so it needs no scanner token.
    // The other forms are aliased to it: a dotted receiver, and a first
    // argument that is a `new`, an array type, a name, such a call in
    // parentheses or a literal that the argument goes on past
    // (`println 'a' + b`), in `_path_command_chain`, and a call that is a
    // value in `_value_command_chain`. After a comma an argument is a named
    // argument, any expression or a spread argument (`foo 'a', *args`):
    // Apache's `argumentListElement`. The continuations of a chain
    // (`foo a bar`) are not taken. Closes a subset of murtaza64 #5.
    command_chain: $ => prec(2, seq(
      field('receiver', $.identifier),
      field('argument', choice($._command_argument, alias($._command_named_argument, $.named_argument))),
      commandArgumentsAfterFirst($),
    )),

    // §5.5 — a parenthesis-free call as the value of a declarator or an
    // assignment (`def out = sh script: 'make', returnStdout: true`),
    // aliased to `command_chain`. A newline is whitespace, so the call
    // opens with `_command_value_start`, which the scanner emits only
    // when a literal, a `name:`, `new`, an array type value or such a call
    // in parentheses follows the receiver on its line, or a name, `!` or
    // `~` after a receiver that may not name a type (§6.14).
    // Without it `def x = foo` followed by `1` on the next line would be
    // one call. A positional first argument is a whole expression, as in
    // Apache (`def x = foo 'a' + b`); the token makes it start with one
    // of those. A closure is not a first argument here: `def c = foo { }`
    // stays a `method_invocation`, as Apache reads it. `prec.right` lets
    // a `,` continue the call, as in Groovy: `def x = foo 1, y = 2`
    // passes `y = 2` to `foo`. The receiver may have member accesses
    // (`def r = obj.method 1`), which the scanner reads with the name.
    _value_command_chain: $ => prec.right(seq(
      $._command_value_start,
      field('receiver', choice($.identifier, $._command_path)),
      commandArguments($),
    )),

    // `enhancedStatementExpression` as one symbol, for the condition of `if`,
    // `while` and `do`, a `switch` value and a `synchronized` lock. Inline,
    // its choice would give each alternative its own copy of the states for
    // the rest of the rule, about 70 in all, most for the `else` of an `if`.
    // The symbol costs a reduction per condition instead, about 5% more
    // instructions on code made only of conditions. A parenthesized
    // expression and an interpolation end right after the choice, so they
    // keep it inline: there it costs few states, and an extra reduction in
    // every parenthesis cost about 4%.
    _statement_expression: $ => enhancedStatementExpression($),

    // §5.5 item 2 — a statement whose receiver has a member access
    // (`obj.method 1`, `a?.b.c x: 1`). Unlike the identifier form above it
    // holds a scanner token between the receiver's first name and its
    // first access, emitted only when a first argument of the value form
    // follows the receiver on its line, so `result.size` followed by `2`
    // on the next line stays two statements. The arguments are those of
    // the value form. A value with such a receiver is
    // `_value_command_chain`: a name in a value is an `_expression`
    // otherwise, and referencing it here as well would cost every value a
    // reduction.
    //
    // A statement whose receiver is one name and whose first argument
    // starts with `new`, is an array type, a name or such a call in
    // parentheses (`foo new Bar()`, `foo String[].class`, `println x`,
    // `println(foo 1)`) opens with `_command_statement_start`. The
    // grammar cannot read it from the name alone: `foo` followed by
    // `new Bar()` or `x` on the next line is two statements, and the
    // scanner decides before the name, where no comment can hide that line
    // break. So does one whose first argument is a literal that the
    // argument goes on past (`println 'a' + b`, `foo 1` then `.m()` on the
    // next line), while `foo 1` then `-x` stays two statements (§6.14).
    _path_command_chain: $ => seq(
      choice(
        field('receiver', $._statement_command_path),
        seq($._command_statement_start, field('receiver', $.identifier)),
      ),
      commandArguments($),
    ),

    // The receivers with member accesses of a value and of a statement.
    // The scanner reads them (§6.14, §6.24), so their accesses take names
    // only.
    ...commandPathRules('_command_path', $ => field('object', $.identifier)),
    ...commandPathRules('_statement_command_path', $ => field('object',
      $._statement_command_object)),

    // The object of a statement receiver's first access: its first name,
    // or (§6.24) a call of that name with a closure (`foo { }.bar 1`, Groovy's
    // `foo({ }).bar(1)`), which `_closure_command_start` opens where the
    // scanner has read the accesses and a first argument after them. One
    // rule for both keeps the states of the accesses shared.
    _statement_command_object: $ => choice(
      seq($.identifier, $._command_path_start),
      alias($._closure_command_call, $.method_invocation),
    ),

    _closure_command_call: $ => seq(
      $._closure_command_start,
      field('function', $.identifier),
      field('closure', $.closure),
    ),

    // §5.5 item 3 — named arguments in a parenthesis-free call:
    // `exclude group: 'a', module: 'b'`, `archiveArtifacts artifacts: 'x'`,
    // GORM's `name blank: false, size: 1..100`. `_command_named_key`
    // is an empty token that the external scanner emits before a name
    // only when `:` follows it on the same line, so `Type name = …` and
    // a bare statement followed by a Spock label on the next line are
    // unaffected. The name itself is an ordinary `identifier`. A
    // keyword is a valid key (Apache `namedArgLabel`): it has no
    // action after the empty token, so it lexes as an identifier, and
    // the empty `property` set does the same for the reserved `new`.
    //
    // The key `var`, `yield`, `in` or `as` is a token of its own that
    // spans the word, aliased to `identifier`. After `it`, the scanner
    // reads past the word to tell `it in: 1` from `it in xs`, and the
    // keyword it may emit instead must end at the word, where an empty
    // key cannot (§6.16).
    //
    // A string key (`attributes 'Main-Class': 'a.B'`, Apache's
    // `namedArgPrimary`) needs no token: no other argument has a `:`
    // after a string, so the precedence takes the `:` from the string as
    // an expression.
    _command_named_argument: $ => choice(
      seq(
        choice(
          seq($._command_named_key, field('key', reserved('property', $.identifier))),
          field('key', alias($._contextual_key, $.identifier)),
        ),
        ':',
        field('value', $._expression),
      ),
      prec.right(1, seq(field('key', $.string_literal), ':', field('value', $._expression))),
    ),

    // After a comma a newline cannot end the statement, so a key on a
    // continuation line (`sh script: 'make',` then `returnStdout: true`)
    // needs no scanner token. Same-line keys normally come from the
    // scanner; `default`, which the scanner rejects, reaches this rule
    // on the same line too (`foo 'a', default: 1`). The reserved `case`
    // (see `reserved`) is no identifier here, so it is a key of its own.
    //
    // A call that is a value can sit inside a list or an argument list,
    // where a name and `:` after its comma may also be the next map
    // entry or named argument: `[x = foo a: 1, b: 2]`. The precedence
    // gives it to the call, as Apache's greedy argument list does, and
    // `right` keeps `b: c = 1` one argument.
    _command_continued_named_argument: $ => prec.right(1, seq(
      field('key', choice($.identifier, alias('case', $.identifier))),
      ':',
      field('value', $._expression),
    )),

    _command_argument: $ => choice(
      $.string_literal,
      $.number_literal,
      $.boolean_literal,
      $.null_literal,
      $.closure,
    ),

    // §5.3 — `label: stmt`. Required only at statement-start
    // position; map_entries (which share `:`) are inside `[…]`
    // and don't collide here. `prec(1, ...)` keeps both
    // labeled_statement and expression_statement alive after
    // `identifier`; the `:` decides. A scanner-token form
    // (`_label_colon`) was attempted but interacted with
    // named-argument command chains (`archiveArtifacts artifacts: 'x'`),
    // which now have their key decided by `_command_named_key`
    // instead; see the §6.5 row in `docs/divergences-from-spec.md`.
    //
    // `default` is a keyword at the start of a statement, so it is a
    // label by its own alternative, and `foo default: 1` stays a
    // statement and a label (spec §6.8). In a switch body a `default`
    // after a case ends the case instead (`switch_case`).
    //
    // A labeled block (`lbl: { … }`) is Groovy's way to make braces a
    // block, and so it is here (`statementBlock`).
    labeled_statement: $ => {
      const statement = choice(
        field('statement', $._statement),
        statementBlock($, (rule) => field('statement', rule)),
      );
      return choice(
        prec(1, seq(field('label', $.identifier), ':', statement)),
        seq(field('label', alias('default', $.identifier)), ':', statement),
      );
    },

    // §4 — block of statements: control-flow, method and constructor
    // bodies, static / instance initializers, and a `{` that opens a
    // statement (`statementBlock`). Shares `{ ... }` syntax with closure
    // (§5.8). A `prec(1, …)` keeps block preferred at use sites where
    // both `block` and a closure-bearing `_statement` are valid
    // alternatives (e.g. the body slot of `if` / `while`).
    block: $ => prec(1, seq(
      '{',
      statementList($),
      '}',
    )),

    // §4 — if / else. `prec.right` resolves the dangling-else
    // ambiguity in favour of the closest `if`. Body accepts either
    // an explicit block (preferred) or a single statement. The
    // `optional(';')` after the consequence keeps the else attached
    // when a braceless consequence is terminated by a semicolon
    // (`if (x) a(); else b()`); without it the `;` is claimed by the
    // enclosing statement list (§5.10) and the `else` is severed. It
    // sits outside the else clause so the no-else form (`if (x) a();
    // b()`) does not require an `else` after eating the `;`.
    if_statement: $ => prec.right(seq(
      'if',
      '(',
      field('condition', $._statement_expression),
      ')',
      field('consequence', $._if_consequence),
      optional(';'),
      optional(seq(
        'else',
        field('alternative', choice($._body_block, $._statement)),
      )),
    )),

    // §4 — while loop. do-while lands separately.
    while_statement: $ => seq(
      'while',
      '(',
      field('condition', $._statement_expression),
      ')',
      field('body', choice($._body_block, $._statement)),
    ),

    // §4 — C-style for. init / condition / update slots all
    // optional; bare `for (;;)` is the infinite-loop idiom. The init
    // slot accepts a single expression (`i = 0`) or a variable
    // declaration (`int i = 0`, `def i = 0`, incl. multiple
    // declarators). Apache `GroovyParser.g4` `forInit:
    // localVariableDeclaration | expressionList` also allows a
    // comma-separated expression list (`i = 0, j = 1`); that form is
    // not yet supported.
    for_statement: $ => seq(
      'for',
      '(',
      optional(field('init', choice(
        $._expression,
        $.local_variable_declaration,
        $.multi_assignment_declaration,
      ))),
      ';',
      optional(field('condition', $._expression)),
      ';',
      optional(field('update', $._expression)),
      ')',
      field('body', choice($._body_block, $._statement)),
    ),

    // §4 — do-while. Body is required; condition follows the body.
    // `optional(';')` lets a braceless body be terminated before
    // `while` (`do a(); while (c)`, the Java/Groovy `do S while (e);`
    // form); without it the `;` errors since `while` is required next.
    do_while_statement: $ => seq(
      'do',
      field('body', choice($._body_block, $._statement)),
      optional(';'),
      'while',
      '(',
      field('condition', $._statement_expression),
      ')',
    ),

    // §4 — return / break / continue / throw / assert. A line break
    // right after `return`, `break` or `continue` ends the statement, as
    // Apache's `RETURN expression?` and `BREAK identifier?` take no NL
    // before the value or the label: the scanner emits
    // `_jump_statement_end` there (§6.23), so `return` followed by `foo()`
    // on the next line is a bare `return` and a call. `throw` and `assert`
    // still take a value on the next line, which Groovy rejects.
    // A statement may follow a bare `return`, `break` or `continue`, so
    // the scanner offers `_command_statement_start` and
    // `_declaration_start` before a name there (§6.14, §6.21). Taking them
    // here keeps `return foo new Foo()` the return of `foo` followed by the
    // creation, as `return foo 1` is, and `return String s` the return of
    // `String` followed by `s`; `prec.right` prefers the shift, as it does
    // for the name itself. Ending the statement at a token that only the
    // scanner's look-ahead decides would make its extent depend on text
    // past its own tokens, which a reparse that reuses it does not read
    // again; `_jump_statement_end` is the statement's own token, so what
    // the scanner read for it counts against the statement.
    return_statement: $ => prec.right(seq(
      'return',
      optional(choice(
        $._jump_statement_end,
        $._expression,
        seq($._command_statement_start, $.identifier),
        seq($._declaration_start, $._expression),
      )),
    )),

    break_statement: $ => prec.right(seq(
      'break',
      optional(choice(
        $._jump_statement_end,
        seq(
          optional(choice($._command_statement_start, $._declaration_start)),
          field('label', $.identifier),
        ),
      )),
    )),

    continue_statement: $ => prec.right(seq(
      'continue',
      optional(choice(
        $._jump_statement_end,
        seq(
          optional(choice($._command_statement_start, $._declaration_start)),
          field('label', $.identifier),
        ),
      )),
    )),

    throw_statement: $ => seq('throw', $._expression),

    // §4 — `yield expr` in a Groovy 4 switch expression's case
    // body. The scanner emits the keyword only when an expression
    // follows (§6.16), so `yield.foo()` and `yield = 1` use a name. It
    // does so wherever a statement may start, not only in a switch
    // expression, so `yield x` elsewhere stays a yield statement.
    yield_statement: $ => prec.right(seq(
      alias($._yield_keyword, 'yield'),
      $._expression,
    )),

    // §4 — power-assert: `assert e`, `assert e : msg` or `assert e, msg`
    // (Apache `assertStatement` takes `COLON | COMMA`). The colon
    // separator may collide with map_entry in later iterations, but
    // assert is a statement-only construct so the context resolves
    // it.
    assert_statement: $ => prec.right(seq(
      'assert',
      $._expression,
      optional(seq(choice(':', ','), field('message', $._expression))),
    )),

    // §4 — try / catch / finally. At least one of catch/finally is
    // required per Java/Groovy semantics; the grammar relaxes that
    // to allow a bare `try { … }` and lets downstream tooling
    // surface the error if needed.
    try_statement: $ => seq(
      'try',
      optional(field('resources', $.resource_specification)),
      field('body', $.block),
      repeat($.catch_clause),
      optional($.finally_clause),
    ),

    // §4 — `synchronized (lock) { … }` (Apache `#synchronizedStmtAlt`).
    // The body must be a block.
    synchronized_statement: $ => seq(
      'synchronized',
      '(',
      field('lock', $._statement_expression),
      ')',
      field('body', $.block),
    ),

    // §4 — `try (Foo f = expr; Bar b = expr) { … }`. Resources
    // are typed or `def`-style declarators with required `=`
    // initializer (per Java-style try-with-resources). Multiple
    // resources separated by `;`.
    resource_specification: $ => seq(
      '(',
      $.resource,
      repeat(seq(';', $.resource)),
      optional(';'),
      ')',
    ),

    resource: $ => seq(
      optional('final'),
      optional(choice('def', varKeyword($), field('type', $._type))),
      field('name', $.identifier),
      '=',
      field('value', $._expression),
    ),

    catch_clause: $ => seq(
      'catch',
      '(',
      field('parameter', $.catch_formal_parameter),
      ')',
      field('body', $.block),
    ),

    // §4 — `catch_formal_parameter` accepts either a single _type
    // or a multi_type for Java 7+ / Groovy multi-catch. Closes
    // murtaza64 #39. The type itself is optional — Groovy allows a
    // bare `catch (e) { … }`, implicitly typed as Exception.
    catch_formal_parameter: $ => seq(
      optional('final'),
      optional(choice('def', varKeyword($), field('type', choice($._type, $.multi_type)))),
      field('name', $.identifier),
    ),

    multi_type: $ => seq(
      $._type,
      repeat1(seq('|', $._type)),
    ),

    finally_clause: $ => seq(
      'finally',
      field('body', $.block),
    ),

    // §4 — class / trait / interface declarations. v1 supports the
    // shape `[keyword] Name { body }`. extends, implements, generics,
    // modifiers, and sealed / permits clauses land in later
    // iterations. trait closes the dekobon #247 trait row.
    // §4 — class declaration with optional Groovy 4 sealed modifier
    // and `permits` clause.
    class_declaration: $ => seq(
      repeat($.annotation),
      optional($._type_modifiers),
      sealedModifier($),
      'class',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.superclass),
      optional($.super_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    // §6.13 — the keyword comes from the scanner, which emits it only
    // when a trait header follows, so `private String trait` is a field.
    trait_declaration: $ => seq(
      repeat($.annotation),
      optional($._type_modifiers),
      sealedModifier($),
      alias($._trait_keyword, 'trait'),
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.extends_interfaces),
      optional($.super_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    interface_declaration: $ => seq(
      repeat($.annotation),
      optional($._type_modifiers),
      sealedModifier($),
      'interface',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.extends_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    // §4 / §5.14 — generic type parameter list on a class /
    // interface / trait / method declaration. The `<` is
    // `token.immediate` so the parser only treats it as a type
    // parameter opener when it follows the declaration name with
    // no intervening whitespace.
    type_parameters: $ => seq(
      TYPE_ARGUMENTS_LT,
      $.type_parameter,
      repeat(seq(',', $.type_parameter)),
      '>',
    ),

    type_parameter: $ => seq(
      field('name', alias($.identifier, $.type_identifier)),
      optional(seq(
        'extends',
        $._type,
        repeat(seq('&', $._type)),
      )),
    ),

    // §4 — class has a single super and zero-or-more interfaces.
    superclass: $ => seq('extends', $._type),

    super_interfaces: $ => seq(
      'implements',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — trait / interface list their super-interfaces with `extends`.
    extends_interfaces: $ => seq(
      'extends',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — Java-style modifiers. Each one is a hard reserved keyword
    // in Groovy (§5.2). Here it is a keyword only where a modifier can
    // stand; anywhere else the word lexes as an identifier, and input
    // such as `foo(private)` parses with no ERROR (#93).
    _modifier: _ => choice(
      'public',
      'private',
      'protected',
      'static',
      'final',
      'abstract',
      'synchronized',
      'native',
      'transient',
      'volatile',
      'strictfp',
    ),

    // The modifiers of a type declaration, `default` among them
    // (divergence 20). A hidden rule rather than `repeat($._modifier)`
    // in each declaration: the modifiers reduce to it at the
    // declaration keyword, so the header after it has one production
    // whether or not `default` is there.
    _type_modifiers: $ => choice(
      modifierList($),
      seq(optional(modifierList($)), 'default', repeat(choice($._modifier, $.annotation))),
    ),

    // One or more modifiers with annotations among them after the first
    // (`modifierList`).
    _modifiers: $ => seq($._modifier, repeat(choice($._modifier, $.annotation))),

    permits_clause: $ => seq(
      'permits',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — `@interface Foo { … }` declares an annotation type.
    // `@` followed by the `interface` keyword is unambiguous —
    // `interface` is a hard reserved keyword (§5.2) so the
    // alternative parse `annotation` (which needs identifier)
    // cannot fire here.
    annotation_type_declaration: $ => seq(
      repeat($.annotation),
      optional($._type_modifiers),
      '@',
      'interface',
      field('name', $.identifier),
      field('body', alias($._annotation_type_body, $.class_body)),
    ),

    // The member list of `class_body` plus an element with a default.
    // Groovy accepts `default` after the `()` of any abstract method,
    // but only an annotation type element uses it, and only here may it
    // follow the `()`: in an interface, `void a()` then
    // `default void b() { }` on the next line is two methods, which
    // Groovy tells apart by the line break, whitespace here.
    _annotation_type_body: $ => seq(
      '{',
      repeat(choice(
        $._class_member,
        alias($._annotation_type_element, $.method_declaration),
        $.block,
        ';',
      )),
      '}',
    ),

    // §4 — `String value() default 'x'`, Apache's `methodDeclaration`
    // with `DEFAULT elementValue`, which excludes a throws clause. An
    // element with no default is an ordinary `method_declaration`.
    //
    // A `default` after the `()` could also end the member there and
    // start the next one, a method with the `default` modifier. The
    // precedence, equal to the right-associative one of
    // `method_declaration`, keeps it this element's default, on the
    // same line or the next.
    _annotation_type_element: $ => prec(2, seq(
      ...methodHead($),
      'default',
      field('default', $._element_value),
    )),

    // §4 — member list shared by class / interface / trait bodies and
    // the anonymous inner class body of `object_creation_expression`.
    // An `@interface` body and a record body extend it
    // (`_annotation_type_body`, `_record_body`) and are aliased to it.
    // The body accepts `;` as an empty member / terminator the same
    // way statement lists do (§5.10): a field or abstract method is
    // commonly written `int x = 1;` / `void f();`, and a stray `;` is
    // a no-op member. `;` is anonymous, so it never appears in the AST.
    //
    // A bare `block` is an instance initializer. It sits in the member
    // repeat — here, in the four identical repeats of `enum_body` and in
    // the ones of `_annotation_type_body` and `_record_body` — rather
    // than in `_class_member`, because `enum_body` also places
    // a lone `_class_member` directly after its constants. A block in
    // that slot is an LR conflict on `A() {` (constructor, or constant
    // plus initializer) and, once that is resolved, silently accepts
    // `A { }` (a constant with an anonymous class body, unsupported)
    // as constant-plus-initializer.
    class_body: $ => seq(
      '{',
      repeat(choice($._class_member, $.block, ';')),
      '}',
    ),

    // Nested type declarations are members, as in Apache's
    // `memberDeclaration`. Each kind must be listed: a declaration
    // keyword with no action at member start lexes as an identifier,
    // so `class Inner { … }` would silently parse as a field `Inner`
    // of type `class` followed by an instance initializer block, and
    // a nested `record R(int x) { }` as a method `R` returning
    // `record`. The `record`, `trait` and `sealed` keywords come from
    // the scanner (§6.7, §6.13), so listing their declarations here
    // does not reserve the words as member names. No `conflicts` entry
    // is needed.
    _class_member: $ => choice(...classMembers($, $.constructor_declaration)),

    // §4 — constructors share the method_declaration shape but have
    // no return type and the name matches the enclosing class. We
    // don't enforce the name match (downstream tooling can), but we
    // emit a distinct node kind so IDE features (rename, jump) can
    // tell constructors apart from methods. `prec(3, …)` wins over
    // both field and method when the identifier is followed by `(`
    // and no return type precedes it.
    constructor_declaration: $ => constructorDeclaration($, optional(modifierList($))),

    // The constructor form for the member directly after an enum's
    // trailing comma (see `enum_declaration`). Only access modifiers
    // qualify: `static f() { }` there is a method. The weight is above
    // a method with a body, which matches the same text through its
    // modifier-only form.
    _modified_constructor_declaration: $ => weighted(4, constructorDeclaration(
      $,
      repeat1(choice('public', 'private', 'protected')),
    )),

    // §4 — `static { … }` class-body initializer block. Runs once
    // when the class is loaded. Distinct node kind so downstream
    // tools can index it.
    static_initializer: $ => seq(
      'static',
      field('body', $.block),
    ),

    // §4 — class / interface / trait field. Java-style shape: optional
    // annotations, optional modifiers, `def` or a `_type`, then one or
    // more declarators (each with optional initializer). Method
    // declarations share the same prefix; `prec(2, …)` on
    // `method_declaration` and the trailing `(` discriminate.
    //
    // A modifier alone (no `def`, no type) also introduces a field —
    // `static MEDIUM_TTL = 900` infers Object, a common constant-field
    // idiom. Gated behind `repeat1($._modifier)` (not `repeat`) so this
    // alternative can't match a bare, modifier-less identifier, which
    // would collide with a plain expression_statement.
    field_declaration: $ => choice(
      // Prefer the typed field when both parses succeed, rather than
      // splitting `private final int v\n C() {}` after `int`. The
      // weight is above a method body's: the split reading holds a
      // method (`v C() {}`), which would otherwise tie.
      weighted(TYPED_FIELD_WEIGHT, seq(
        repeat($.annotation),
        optional(modifierList($)),
        // `def` is a modifier in Apache's `variableDeclaration`, so a type
        // may follow it (`private def Foo foo`). The scanner opens that
        // type as it opens a typed local's (§6.21), so `def x` keeps its
        // name without a fork at every `def`. `var` is a modifier as well
        // (Apache's `modifier`), which the scanner emits before a name.
        choice(
          choice('def', varKeyword($)),
          field('type', $._type),
          seq(choice('def', varKeyword($)), $._declaration_start,
            field('type', $._type)),
        ),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      )),
      // The negative weight settles the split that the typed weight
      // cannot. `private final int v` followed by `public A() { }` is
      // also `private final int`, a field `v public` and a constructor
      // `A() { }`: a keyword is an identifier wherever it has no
      // action. Both readings hold one typed field, so they tied, and
      // a comment between the two members made the split win.
      weighted(MODIFIER_ONLY_FIELD_WEIGHT, seq(
        repeat($.annotation),
        modifierList($),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      )),
      // Annotations alone, Spock's `@Subject svc = new Svc()`: Apache's
      // `modifiers` takes an annotation as a modifier. As for a local, the
      // first name needs a value (`_placeholder_declarator`).
      seq(
        repeat1($.annotation),
        alias($._valued_declarator, $.variable_declarator),
        repeat(seq(',', $.variable_declarator)),
      ),
    ),

    // §4 — `def name(params) body`, or a return type in place of
    // `def`, with modifiers and a throws clause. The static precedence
    // wins over local_variable_declaration when `(` follows the name,
    // so `def f() { … }` at script-top-level parses as a method
    // rather than a `def f` variable followed by `(…) { … }`.
    // Modifiers alone (no `def`, no return type) also introduce a
    // method outside a class body: `final f() { }`. In a class body
    // that shape stays a constructor_declaration.
    //
    // A method with a body carries a dynamic precedence because its
    // tokens also parse as statements: `String f() { }` as the
    // expression `String` followed by the call `f() { }`, and
    // `def f(a) { }` as a variable `f` followed by the cast
    // `(a) { }`. Without a weight, a comment before the method made
    // the split win. The weight is above the blank final local's
    // (`final Type name`, see local_variable_declaration), but
    // `final String f(a) { }` is also that local followed by the cast
    // `(a) { }`, whose weight makes up the difference: the two tie, and
    // tree-sitter's last resort keeps the method.
    //
    // A method without a body carries a negative one. Its tokens are
    // also `x` followed by the call `foo()`, and outside a class body
    // only that reading is valid Groovy: a script method needs a
    // body. In a class body nothing competes, so abstract and
    // interface methods are unaffected.
    method_declaration: $ => methodDeclaration($, methodHead($)),

    // §4 — a method with the `default` modifier, aliased to
    // `method_declaration` (divergence 20). Apache's `modifier` takes
    // `default` before any declaration; Groovy 3 ignores it except on
    // an interface method. This rule and `_type_modifiers` make
    // `default` a keyword wherever a member or a statement can start,
    // as it is in Groovy, where it is reserved: before them it lexed as
    // an identifier there, and `default void b() { }` was a field of
    // type `default` named `void` and a constructor `b`. With no return
    // type the member is a method, where after other modifiers it is a
    // constructor: the modifier is meaningless on a constructor.
    //
    // The keyword also starts the next member or statement after one
    // that can end, so a line break before it separates the two, as in
    // Groovy. Where a fork between a field's name and a type comes
    // before (`private String y`), the reading that takes `default` as
    // the name of a field survives; the weights of `method_declaration`
    // outrank it.
    _default_method_declaration: $ => methodDeclaration($, methodHead($, seq(
      optional(modifierList($)),
      'default',
      repeat(choice($._modifier, $.annotation)),
      optional(field('type_parameters', $.method_type_parameters)),
      optional(choice('def', field('return_type', $._type))),
    ))),

    // §4 / §5.14 — method-level generic type parameters
    // (`<T> T identity(T x) { x }`). Distinct from `type_parameters`
    // (used on class/trait/interface) because here the `<` cannot be
    // `token.immediate` — it appears at the start of the declaration
    // with optional whitespace from preceding modifiers / annotations.
    method_type_parameters: $ => seq(
      '<',
      $.type_parameter,
      repeat(seq(',', $.type_parameter)),
      '>',
    ),

    // §4 — `throws E1, E2, …` after a method signature.
    throws_clause: $ => prec.left(seq(
      'throws',
      $._type,
      repeat(seq(',', $._type)),
    )),

    formal_parameters: $ => seq(
      '(',
      optional(seq(
        $.formal_parameter,
        repeat(seq(',', $.formal_parameter)),
      )),
      ')',
    ),

    // §5.13 — formal parameter with optional annotations (Spring's
    // `@PathVariable`, `@RequestBody`, JPA's `@Param`, … — ubiquitous
    // on framework-facing methods), optional type, optional varargs
    // marker (`Type... name`), and optional default value
    // (`name = expr`). `final` is the only modifier accepted here.
    // `def` and `var` stand in place of the type and are keywords, so
    // `def f(def a)` has no `type`. The shape is `parameter()`, shared
    // with `closure_parameter`.
    formal_parameter: $ => parameter($),

    varargs_type: $ => seq($._type, '...'),

    // §4 — enum. Closes murtaza64 #36 first half. Constants may
    // carry constructor arguments (`RED(0xff0000)`) via the optional
    // argument_list.
    //
    // The body follows GroovyParser.g4 `classBody` for `t == 2`:
    // after the constants either a trailing `,` closes the list, or
    // members follow after `;`, `,;`, or nothing at all — Groovy's
    // own parser treats the newline as the separator (GROOVY-7773,
    // GROOVY-9306), and newlines are extras here, so the member
    // simply follows the last constant. With no constants the body
    // is an ordinary member list.
    //
    // Groovy up to 4.x also accepts a member directly after a trailing
    // `,` (`A, B,\n private int x`): GroovyParser.g4 on those branches
    // has `enumConstants (nls COMMA)? sep?` before the member list.
    // Groovy 5 requires a `;`. Without this alternative the modifier
    // had no action after the `,`, lexed as an identifier, and became
    // a constant named `private` with no ERROR.
    //
    // The alternative is narrower than Groovy, which takes any member
    // there (divergence 13). `prec(1)` on `enum_constant` keeps a bare
    // identifier after the `,` a constant, so the member has to start
    // with something that cannot be one: a modifier, `def`, a
    // declaration keyword, or an annotation followed by one of those.
    // `A,\n String x` is an ERROR, and so is an instance initializer.
    // A constructor needs an access modifier. `B(x) { … }` after a
    // `,`, with or without annotations, reads as a constant with an
    // anonymous class body (unsupported, so an ERROR). With an access
    // modifier it cannot be a constant, and with no constructor form
    // here `private B(x) { }` parsed, with no ERROR, as a method named
    // `B`: the modifier-only form of `method_declaration` took it.
    // Other modifiers do not make a constructor, so `static f() { }`
    // is a method.
    //
    // An instance initializer (`block`) is accepted wherever the
    // member list repeats — after `;`, after a first member, or with
    // no constants — but not directly after a constant. Newlines are
    // extras, so `A\n{ … }` (in the .g4 a constant, then an
    // initializer) cannot be told from `A { … }` (a constant with an
    // anonymous class body, unsupported); both stay errors.
    //
    // The four member repeats are spelled out, and must stay
    // identical to the one in `class_body`, so that the generator
    // folds all five into one aux symbol (`aux_sym_class_body_repeat1`).
    // A differing copy, or a hidden rule that is not inlined, adds a
    // symbol and renumbers every id after it — the alias kinds
    // `named_argument`, `quoted_identifier` and `type_identifier`
    // included — which a patch release must not do.
    enum_declaration: $ => seq(
      repeat($.annotation),
      optional($._type_modifiers),
      'enum',
      field('name', $.identifier),
      optional($.super_interfaces),
      field('body', $.enum_body),
    ),

    enum_body: $ => seq(
      '{',
      choice(
        seq(
          $.enum_constant,
          repeat(seq(',', $.enum_constant)),
          optional(choice(
            ',',
            seq(optional(','), ';', repeat(choice($._class_member, $.block, ';'))),
            seq($._class_member, repeat(choice($._class_member, $.block, ';'))),
            seq(
              ',',
              choice(...classMembers(
                $,
                alias($._modified_constructor_declaration, $.constructor_declaration),
              )),
              repeat(choice($._class_member, $.block, ';')),
            ),
          )),
        ),
        repeat(choice($._class_member, $.block, ';')),
      ),
      '}',
    ),

    // §4 — `prec(1)` makes a bare identifier at the start of the body
    // a constant rather than the type of the first member: `A\n E() { }`
    // is a constant followed by a constructor, not a method `E`
    // returning `A`. The mirror case with no constants, `int f() { }`
    // as the first thing in the body, therefore reads as constant `int`
    // plus constructor `f` — a newline is whitespace here and nothing
    // else tells the two apart; a modifier, `def`, or a leading `;`
    // makes it a member. Static precedence rather than `prec.dynamic`
    // with an `[enum_constant, _type]` conflict: both yield the same
    // tree on every valid shape, but the GLR form silently accepts
    // `A\n B` (constants missing their comma, which Groovy rejects) as
    // a field `A B`, where this form reports the ERROR. Annotations
    // share the prefix with an annotated member; the same precedence
    // decides after them, so `@Foo A\n int x` is a constant and a
    // field.
    enum_constant: $ => prec(1, seq(
      repeat($.annotation),
      field('name', $.identifier),
      optional(field('arguments', $.argument_list)),
    )),

    // §4 — record (Groovy 4+), after Apache's `classDeclaration`:
    // `[modifiers] record Name[<T>](Type a, Type b) [implements I] [{ body }]`.
    // Records cannot `extends`, and the body is optional here where
    // Groovy requires it.
    // §5.2 / §6.7 — `record` is a contextual keyword. The keyword
    // comes from the external scanner, which emits it only when a
    // name follows on the same line and `(` or `<` after that; in
    // every other position the word lexes as `identifier`. A grammar literal
    // cannot do this: keyword extraction yields the keyword wherever
    // it has an action, which made `record.key()` at statement start,
    // and `private Record record` once record_declaration became a
    // class member, errors. The alias keeps the anonymous `record`
    // node that the queries capture.
    //
    // The dynamic precedence settles a GLR tie the scanner creates.
    // After `private Record`, the `[variable_declarator, _type]`
    // conflict forks on the next word: `record` is the field's name
    // in one branch and a type in the other. When a record header
    // follows on the next line, the first branch lexes its `record`
    // as the keyword and the second as that field's name, and both
    // survive (`private Record` / `record record` / constructor
    // `Inner(int x) { }` is a valid member list). Without a weight,
    // which tree wins depends on tree shape, and a comment between
    // the two lines flipped it. Favouring the record declaration is
    // right whenever both parse.
    record_declaration: $ => weighted(1, prec.right(seq(
      repeat($.annotation),
      optional($._type_modifiers),
      alias($._record_keyword, 'record'),
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      field('components', $.record_components),
      optional($.super_interfaces),
      optional(field('body', alias($._record_body, $.class_body))),
    ))),

    // The member list of `class_body` plus a compact constructor, which
    // no other body takes. A type nested in a record keeps its own body
    // rule, and a record nested in a class brings this one.
    _record_body: $ => seq(
      '{',
      repeat(choice(
        ...memberDeclarations($, $.constructor_declaration),
        $._record_type_member,
        $.compact_constructor_declaration,
        $.block,
        ';',
      )),
      '}',
    ),

    // A type nested in a record, weighted above the split of its header
    // into fields and a compact constructor (RECORD_TYPE_MEMBER_WEIGHT).
    // Each type is aliased to itself because the generator drops the
    // reduction of a hidden rule with one unaliased child, and the
    // weight with it.
    _record_type_member: $ => weighted(RECORD_TYPE_MEMBER_WEIGHT, choice(
      ...typeDeclarations($).map(type => alias(type, type)),
    )),

    // §4 — a record's compact canonical constructor, Apache's
    // `modifiersOpt compactConstructorDeclaration` (`methodName nls
    // methodBody`): `public Warning { … }`. Apache's parser takes it in
    // any class body and rejects it outside a record afterwards, as it
    // does a name that differs from the record's; the name is not
    // compared here. Apache's modifiers include `def`, which this
    // grammar places after the others, as on a field or a method.
    //
    // The static precedence makes the `{` after modifiers or `def` and
    // a name open this body rather than end a field before an instance
    // initializer. A line break is whitespace, so the `{` on the next
    // line reads the same. Groovy rejects an instance initializer in a
    // record, so only this reading can be valid.
    //
    // The dynamic precedence decides where the `[variable_declarator,
    // _type]` conflict keeps two readings alive, as in `static count`
    // then `R { }`; see COMPACT_CONSTRUCTOR_WEIGHT for its value.
    compact_constructor_declaration: $ => weighted(COMPACT_CONSTRUCTOR_WEIGHT, prec(1, seq(
      repeat($.annotation),
      optional(modifierList($)),
      optional('def'),
      field('name', $.identifier),
      field('body', $.block),
    ))),

    record_components: $ => seq(
      '(',
      optional(seq(
        $.record_component,
        repeat(seq(',', $.record_component)),
      )),
      ')',
    ),

    record_component: $ => seq(
      field('type', $._type),
      field('name', $.identifier),
    ),

    // §4 — package and import. Trailing `;` is optional in Groovy;
    // we accept both forms by not requiring it.
    package_declaration: $ => seq(
      repeat($.annotation),
      'package',
      field('name', $.qualified_name),
    ),

    // §4 — `import [static] qualified.Name [.*] [as Alias]`.
    // `.*` is a single anonymous token so it lexes distinctly from
    // a continuation of qualified_name (`.identifier`). Wildcard
    // imports only appear in this position, so the lexer-wide
    // longest-match is safe.
    import_declaration: $ => seq(
      repeat($.annotation),
      'import',
      optional('static'),
      field('name', $.qualified_name),
      optional(token('.*')),
      optional(seq(alias($._as_keyword, 'as'), field('alias', $.identifier))),
    ),

    qualified_name: $ => seq(
      $.identifier,
      repeat(seq('.', $.identifier)),
    ),

    // §4 / §5.12 — `def x = …`, `var x = …`, and `Type x = …`.
    // Multiple declarators per statement (`def a = 1, b = 2`) are
    // supported in all three forms. `final` is the only modifier
    // accepted here. `final x = 1` with no type or `def` is the
    // modifier-only form; the variable is dynamically typed, as with
    // `def`. `final` also allows the typed form without an initializer
    // (`final String name`, a blank final). After the first declarator
    // of a typed form any may go without one (`String a = 1, b`).
    //
    // A typed form without `final` (`String s`, `int i;`,
    // `List<String> a, b = []`, `String s = 'a'`) opens with
    // `_declaration_start`, which the scanner emits before the type
    // when it reads such a declaration (§6.21), where a statement starts,
    // after its annotations and as a `for` init. Its tokens are also
    // `String` followed by the statement `s` (or `s = 'a'`), and a line
    // break after the type or the name ends the statement in Groovy,
    // which only the scanner sees. Apart from this form only `return`,
    // `break` and `continue` take the token (see return_statement), so
    // the form needs no weight, and where the scanner emits it the
    // declaration and the two statements no longer fork.
    //
    // The other typed forms carry a dynamic precedence because their tokens
    // also parse another way: `String s = 'a'` as the expression
    // `String` followed by `s = 'a'`, and `final String s = 'a'` as
    // the modifier-only `final String` followed by `s = 'a'`. Without
    // a weight, which tree wins depends on tree shape, and a comment
    // before the declaration flipped it. The blank form ranks above
    // the initialized one, or `final String name` followed by
    // `name = 'a'` would tie with `final String` followed by
    // `name name = 'a'`.
    //
    // Newlines do not end a declaration, so these read wrongly with
    // no ERROR node:
    // - `final x` followed by a line that starts with an identifier
    //   reads `x` as the type and that identifier as the name. `y = 2`
    //   becomes `final x y = 2`, the same resolution modifier-only
    //   fields get (`static x` then `y = 1`). A typed declaration on that
    //   line is one of its own: `final x` then `String s`,
    //   `String s = 'a'` or `String s = y -> z` declares `x` and `s`, as
    //   the scanner emits `_declaration_start` there (§6.21).
    // - `final String name` followed by a line reading `foo() { }` is
    //   `final String` followed by the method `name foo() { }`. A
    //   method with a body outranks the blank form, so that
    //   `final String f(a) { }` stays a method (see method_declaration).
    // - A bare `x` followed by a line reading `y = 1` is the typed
    //   declaration `x y = 1`.
    //
    // Annotations may come first on every form (Apache
    // `variableModifier`): `@Field def x = 1` in a script,
    // `@SuppressWarnings('unchecked') List<String> xs = []` in a body.
    local_variable_declaration: $ => prec.right(choice(
      seq(
        repeat($.annotation),
        optional('final'),
        choice('def', varKeyword($)),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      weighted(TYPED_LOCAL_WEIGHT, seq(
        repeat($.annotation),
        optional('final'),
        field('type', $._type),
        $._initialized_declarator,
        repeat(seq(',', $._typed_declarator)),
      )),
      seq(
        repeat($.annotation),
        'final',
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      // A statement may follow `final Type`, a declaration of `Type`, so
      // the scanner may emit `_command_statement_start` before the name
      // (§6.14); `final String s new Foo()` still declares `s`.
      weighted(2, seq(
        repeat($.annotation),
        'final',
        field('type', $._type),
        optional($._command_statement_start),
        field('name', $.identifier),
      )),
      // With more names the blank final weighs what the typed form with an
      // initializer does. A later name's value may hold a lambda
      // (`final String a, b = (x) -> y`), which LAMBDA_WEIGHT ranks against
      // that weight: at 2, `{ final String a, b = (T) (x, y) -> z }` tied
      // the closure with two parameters, and the declaration won.
      weighted(TYPED_LOCAL_WEIGHT, seq(
        repeat($.annotation),
        'final',
        field('type', $._type),
        optional($._command_statement_start),
        field('name', $.identifier),
        repeat1(seq(',', $._typed_declarator)),
      )),
      weighted(2, seq(
        repeat1($.annotation),
        alias($._placeholder_declarator, $.variable_declarator),
      )),
      // Annotations with no `def`, modifier or type (`@Field FOO = [:]` in
      // a script), as Apache's `variableModifiers variableDeclarators`
      // takes them. The first name needs a value, for the reason
      // `_placeholder_declarator` gives.
      seq(
        repeat1($.annotation),
        alias($._valued_declarator, $.variable_declarator),
        repeat(seq(',', $.variable_declarator)),
      ),
      // `def String s = 'a'`: `def` is a modifier before the type, as in
      // Apache's `variableDeclaration`, and the scanner decides the type
      // after it as at a statement's start, so `def String` then `s = 1`
      // on the next line stays two statements (§6.21). `var` is one too
      // (Apache's `modifier` lists `VAR`): `var String s = 'a'`.
      seq(
        repeat($.annotation),
        optional(seq(optional('final'), choice('def', varKeyword($)))),
        $._declaration_start,
        field('type', $._type),
        $._typed_declarator,
        repeat(seq(',', $._typed_declarator)),
      ),
    )),

    // A declarator of a typed form, with or without an initializer. The
    // name and the value are fields of the declaration, as in
    // `_initialized_declarator`.
    _typed_declarator: $ => seq(
      field('name', $.identifier),
      optional(seq('=', field('value', enhancedStatementExpression($)))),
    ),

    // `@Library('shared') _`, the Jenkins idiom: an annotation needs a
    // declaration to stand on, and `_` is the name used for it. Apache
    // takes any name there (`variableModifiers variableDeclarators`).
    // Only `_` is accepted here, as a literal: with any identifier,
    // `@A x` followed by `y = 1` on the next line would also be a typed
    // local `x y = 1`, and nothing but the line break tells them apart.
    _placeholder_declarator: $ => field('name', alias('_', $.identifier)),

    _initialized_declarator: $ => seq(
      field('name', $.identifier),
      '=',
      field('value', enhancedStatementExpression($)),
    ),

    variable_declarator: $ => seq(
      field('name', $.identifier),
      optional(seq('=', field('value', enhancedStatementExpression($)))),
    ),

    _valued_declarator: $ => seq(
      field('name', $.identifier),
      '=',
      field('value', enhancedStatementExpression($)),
    ),

    // The names of a multi-assignment declaration, Apache `typeNamePairs`.
    // A typed name is a `variable_declarator` with a `type` field, so that
    // each name keeps its own type. A keyword before a name lexes as a
    // type: `def (def a, b)` has the type `def`.
    //
    // Groovy 6 adds a rest binding, `def (a, *b) = list`, a node of its
    // own, `rest_declarator`, with the optional `type` and the `name` of
    // a typed name, and keyed pairs, `def (name: n, age: int a) = map`,
    // whose key is a `key` field. Groovy's list holds names or keyed
    // pairs, not both (`typeNamePair (COMMA typeNamePair)*` or
    // `keyedPair (COMMA keyedPair)*`). Here one list takes both, so
    // `def (name: n, b) = m` parses, which Groovy rejects: with a list of
    // pairs alone, recovery from an error after the `=` could not go back
    // into the list at a plain name (`def (k: a, j: b) =, b) = …`), and a
    // run of such errors took time quadratic in its length
    // (`_multi_assignment_tail`). Groovy accepts one rest binding per
    // list, in any place; that is checked after parsing, here not at all.
    //
    // The names are a rule, so that every list, of one name or of more,
    // ends in one parse state. Two recoveries from an error in the list
    // (`def (a::, b) = l` is the name `a` before two skipped tokens, and
    // the key `a` before two others) then meet at the `)`, where the
    // parser keeps one. Written out where they are used, a name and a
    // `repeat` are two rules, `(a)` and `(a, b…)`, with a state each: the
    // recoveries met after the value, at the first token of the next
    // statement, and a reparse that reused that statement kept the other
    // one from there on (`docs/divergences-from-spec.md` §29 has the
    // counts).
    //
    // `prec.right` makes the rule take every `,` after a name, where the
    // rules that use it take names after it as well
    // (`multiAssignmentNames`). A left-recursive rule (`list , name`) has
    // the one state too, and a `,` goes on with it without those names,
    // but it nests a node for each name, where the runtime balances a
    // `repeat`: `ts_node_child`, `ts_node_next_sibling` and
    // `ts_tree_get_changed_ranges` took time quadratic in the names (2 s
    // against 0.02 s for the siblings of 8,000 names). The price is a
    // reparse that differs from a parse from scratch: the runtime reuses
    // this rule's node as a whole, also with an `ERROR` in it, and reused
    // no node of the left-recursive rule after an error (§29).
    //
    // The rule stands before the rules that use it. Tree-sitter sums up
    // the children of a rule in the order of the rules, and on its first
    // pass a hidden rule further down has none, which made the names of a
    // `multi_assignment_declaration` optional in `node-types.json`.
    _multi_assignment_list: $ => prec.right(seq(
      multiAssignmentName($),
      repeat(seq(',', multiAssignmentName($))),
    )),

    // §4 — `def (a, b) = expr`, Apache `variableDeclaration` with
    // `typeNamePairs`. Closes murtaza64 #22. The form without `def` is
    // `multi_assignment_expression`. It takes annotations, then `def`,
    // `final def`, `final var`, `final` alone or `var` alone. Apache
    // takes any modifiers, and annotations alone. Like a local here it
    // does not take `static` and the rest, and it does not take
    // annotations alone: in `@A (a, b) = …` the `(` would open the
    // annotation's arguments or the names, a fork at every annotation
    // with arguments where a statement starts.
    //
    // `var` alone has its own token: `var(a, b)` and `var (a, b).foo()`
    // call a method named `var`, so the scanner emits it only when names,
    // `)` and `=` follow (§6.16). After `final` the `var` before `(` is
    // always the keyword, as it was before `var` could be a name.
    multi_assignment_declaration: $ => seq(
      repeat($.annotation),
      choice(
        seq(optional('final'), 'def'),
        seq('final', optional(varKeyword($))),
        alias($._var_names_keyword, 'var'),
      ),
      $._multi_assignment_tail,
    ),

    // The same in a type body, Apache `fieldDeclaration`, with the
    // modifiers of a field. Groovy parses it and then drops it: it
    // declares no field. `var` opens one with the token of `var` alone
    // (`private var (a, b) = l`). It is not a keyword before a name in a
    // body, where `var x = 1` is a field of type `var`.
    _multi_assignment_field: $ => seq(
      repeat($.annotation),
      choice(
        seq(optional(modifierList($)), choice('def', alias($._var_names_keyword, 'var'))),
        modifierList($),
      ),
      $._multi_assignment_tail,
    ),

    // The names in their parentheses, then the `=` and the value, which
    // stand in this rule, not after it in the two declarations. A rule of
    // the names and their parentheses is complete at its `)`, and the
    // parser reduces it before the `=`. Recovery from an error after the
    // `=` (`def (a, b) =, b) = …`) could then no longer go back into the
    // list, as it did at v0.6.0: it skipped tokens, and a run of such
    // errors took time quadratic in its length
    // (`docs/divergences-from-spec.md` §29). Here the parentheses stay on
    // the stack until the value ends, and a `,` after the `=` goes on
    // with the names after the list. No tree without an `ERROR` changes.
    //
    // The second alternative holds the error productions, for a
    // declaration whose value does not come (`_multi_assignment_valueless`).
    // A `,` may follow one. Recovery otherwise went back into the list at
    // that `,` and took the next line as names (`def (a, b) =,` before
    // `def c = 1` or `class D { }`), which left both lines an `ERROR`;
    // on one line, where a name and a `)` follow, it still goes back
    // into the list, as the run above needs.
    _multi_assignment_tail: $ => choice(
      seq(
        ...multiAssignmentNames($),
        '=',
        field('value', enhancedStatementExpression($)),
      ),
      seq($._multi_assignment_valueless, optional(',')),
    ),

    // Error productions: `_missing_value` never comes, so only recovery
    // takes them, with a missing `identifier` as the value.
    //
    // The first is a declaration whose `=` does not come, as while it is
    // typed (`def (a, b)` before the next statement). Its names are a rule
    // of their own, `_multi_assignment_names`, which the parser reduces at
    // the `)` only before `_missing_value`: in recovery, which tries every
    // reduction. Recovery then has two ways on, besides going back into
    // the list. Where the next token can start a statement, it completes
    // the declaration with the missing value, and the statements after it
    // stay statements. Where it cannot (`def (a, b) : l`), the runtime
    // tries no missing token, and recovery wraps the names, as one node,
    // in an `ERROR` and goes on after `def`, as it did while the names
    // were a rule before the `=`. With the parentheses on the stack alone
    // it had neither way, and took the statements after the declaration
    // into one `ERROR`. Only this production has the weight
    // (`MISSING_VALUE_WEIGHT`).
    //
    // The second is a declaration whose `=` comes and whose value does
    // not, for the `,` that may follow in `_multi_assignment_tail`. Before
    // a token that can end a statement the runtime inserts a missing
    // `identifier` of its own, as the value of the first alternative
    // there. It is a rule with the first, not a missing value in
    // `_multi_assignment_tail` before the `,`: the runtime inserts a
    // missing token only where the next token then reduces a rule.
    _multi_assignment_valueless: $ => choice(
      weighted(MISSING_VALUE_WEIGHT, seq(
        $._multi_assignment_names,
        field('value', alias($._missing_value, $.identifier)),
      )),
      seq(
        ...multiAssignmentNames($),
        '=',
        field('value', alias($._missing_value, $.identifier)),
      ),
    ),

    _multi_assignment_names: $ => seq(...multiAssignmentNames($)),

    // A token that matches no input, for the error productions above. It
    // is no second use of `$._unreachable`: that token's one alias,
    // `switch`, is what a `case` label outside a switch prints as missing,
    // and with two aliases it printed as a named node.
    _missing_value: _ => token(/[^\s\S]/),

    _typed_multi_assignment_declarator: $ => seq(
      field('type', $._type),
      field('name', $.identifier),
    ),

    _rest_multi_assignment_declarator: $ => seq(
      optional(field('type', $._type)),
      '*',
      field('name', $.identifier),
    ),

    _keyed_multi_assignment_declarator: $ => seq(
      field('key', $.identifier),
      ':',
      optional(field('type', $._type)),
      field('name', $.identifier),
    ),

    // §4 / §10 row #37 — Jenkins-style `pipeline { … }` block.
    // Closes murtaza64 #37 (pipeline as a regular statement, not
    // a file-trailer-only construct). The body is a closure, since
    // that's how Jenkins's DSL is shaped — `pipeline` is followed
    // by a closure literal with nested stages, steps, etc.
    pipeline_statement: $ => seq(
      'pipeline',
      field('body', $.closure),
    ),

    // §5.9 — annotation usage. Lands on its own here; class /
    // method / record / enum declarations gain an optional
    // leading `repeat($.annotation)` so a `@Foo @Bar class X { }`
    // attaches the annotations to the class node.
    annotation: $ => seq(
      '@',
      field('name', $.qualified_name),
      optional(field('arguments', alias($._annotation_arguments, $.argument_list))),
    ),

    // Apache's `elementValues` is one element value or a list of pairs.
    // This takes whatever a call takes as well (`@Foo(1, 2)`,
    // `@Foo(x: 1)`), so that an annotation's arguments parse as before,
    // and adds element values that are not expressions.
    _annotation_arguments: $ => argumentList(choice(
      $._element_value,
      alias($._element_value_pair, $.assignment_expression),
      alias($.map_entry, $.named_argument),
      $.spread_arguments,
    )),

    // `name = value`, with the node, fields and command chain that
    // `assignment_expression` has in a call. The name stays unreduced,
    // as in `_statement_assignment`: reducing it to an expression on
    // `=` would conflict with this rule, and the precedence shifts the
    // `=` here instead.
    _element_value_pair: $ => prec.right(PREC.ASSIGN, seq(
      field('left', choice($.identifier, alias('case', $.identifier))),
      field('operator', '='),
      field('right', choice(
        $._element_value,
        alias($._value_command_chain, $.command_chain),
      )),
    )),

    // Apache's `elementValue`: an argument or pair value of an
    // annotation, or an `@interface` element's `default`. It is not an
    // expression, so `foo(@A)` and `[@A]` elsewhere stay errors.
    _element_value: $ => choice(
      $._expression,
      $.annotation,
      alias($._annotation_list, $.list_literal),
    ),

    // Apache's `elementValueArrayInitializer` with annotations in it,
    // `[@A(1), @A(2)]`. A list of expressions is a `list_literal`
    // already; an `@` cannot start an expression, so the two never
    // compete for a token. Apache also takes a mix (`[@A, 1]`) and a
    // nested list (`[[@A]]`), which Groovy rejects when it checks the
    // annotation. A mix would share `[1,` with `list_literal` up to the
    // first `@`, a conflict that every list in an annotation would pay.
    _annotation_list: $ => seq(
      choice('[', alias($._subscript_open, '[')),
      $.annotation,
      repeat(seq(',', $.annotation)),
      optional(','),
      ']',
    ),

    // §4 — `for (x in xs)`, plus the Java-compatible enhanced-for
    // form Groovy accepts side by side: `for (Type x : xs)` / `for
    // (def x : xs)`. The `in` / `:` keyword here is a structural
    // delimiter, not the membership_expression operator or a
    // labelled_statement colon — context disambiguates because the
    // for-header is not an expression position. The `in` is the
    // operator's scanner token (§6.16), so the loop variable and an
    // expression in a classic `for` init still fork on one token.
    for_in_statement: $ => seq(
      'for',
      '(',
      optional('final'),
      optional(choice('def', varKeyword($), field('type', $._type))),
      field('variable', $.identifier),
      choice(alias($._in_keyword, 'in'), ':'),
      field('value', $._expression),
      ')',
      field('body', choice($._body_block, $._statement)),
    ),

    // A statement that is `name = value` keeps the name unreduced.
    // A closure can open with the same tokens as a parameter with a
    // default (`{ a = 1 -> a }`), and so can an `if` / `while` / `for`
    // block, because its `{` may also open a closure. With `name`
    // already reduced to an expression, the two readings needed a
    // fork that ran to the end of the value: 14% to 32% more work for
    // `ext { version = '1.0' }` or `if (c) { a = 1 }`, and twice the
    // work for a long value. Here both readings shift the `=` and one
    // stack parses the value; the token after it decides.
    //
    // The form is limited to the start of a statement. Inside an
    // expression it would take `b = 1` out of `a + b = 1`, which the
    // operator precedence reads as `(a + b) = 1`.
    expression_statement: $ => choice(
      $._expression,
      alias($._statement_assignment, $.assignment_expression),
    ),

    _statement_assignment: $ => prec.right(PREC.ASSIGN, seq(
      field('left', $.identifier),
      field('operator', '='),
      field('right', enhancedStatementExpression($)),
    )),

    // §3.2 PRIMARY — minimal alternative set for now. Every primary
    // expression form from §3.2 / §5 will land in this choice over
    // subsequent iterations.
    _expression: $ => choice(
      $.identifier,
      $.number_literal,
      $.string_literal,
      $.boolean_literal,
      $.null_literal,
      $.parenthesized_expression,
      $.unary_expression,
      $.list_literal,
      $.map_literal,
      $.closure,
      $.binary_expression,
      $.range_expression,
      $.identity_expression,
      $.spaceship_expression,
      $.regex_find_expression,
      $.regex_match_expression,
      $.ternary_expression,
      $.elvis_expression,
      $.logical_implication_expression,
      $.assignment_expression,
      $.multi_assignment_expression,
      $.power_expression,
      $.subscript_expression,
      $.safe_subscript_expression,
      $.method_invocation,
      $.update_expression,
      $.unary_update_expression,
      $.field_access,
      $.safe_navigation_expression,
      $.safe_chain_dot_expression,
      $.spread_dot_expression,
      $.method_pointer_expression,
      $.direct_field_access_expression,
      $.safe_direct_field_access_expression,
      $.safe_chain_direct_field_access_expression,
      $.spread_direct_field_access_expression,
      $.method_reference_expression,
      $.object_creation_expression,
      $.array_creation_expression,
      alias($._array_type_value, $.array_type),
      $.parenthesized_type_cast,
      $.cast_expression,
      $.membership_expression,
      $.instanceof_expression,
      $.switch_expression,
      $.lambda_expression,
      alias($._lambda_parenthesized_expression, $.parenthesized_expression),
    ),

    // §4 — Groovy 4 switch-expression. Single rule for both classic
    // `case … :` and arrow `case … -> …` branches; arrow form
    // closes murtaza64 #36 (second half). Used as a statement via
    // expression_statement, or as an expression directly per the
    // §8.2 #36 regression.
    switch_expression: $ => seq(
      'switch',
      '(',
      field('value', $._statement_expression),
      ')',
      field('body', $.switch_block),
    ),

    switch_block: $ => seq(
      '{',
      repeat(choice(
        $.switch_case,
        $.switch_arrow_case,
        $.switch_default,
      )),
      '}',
    ),

    // `default` after a case's statements could also start a statement
    // in them, a method with the `default` modifier or the label
    // `default:`. Left associativity ends the case there, so it is the
    // next branch, as Groovy reads it: no statement in a switch body
    // starts with `default`. `switch_default` ends the same way.
    //
    // A case takes several values, a `value` field each (`case 1, 2 ->`,
    // `case 1, 2:`): Apache's `switchExpressionLabel` is
    // `CASE expressionList[true]` before `->` or `:` in 4.0.x to master.
    // Its `switchLabel`, a switch statement's, takes one expression, but a
    // statement may be a switch expression, which takes the list.
    switch_case: $ => prec.left(seq(
      'case',
      caseValues($),
      ':',
      statementList($),
    )),

    // The arrow body may be terminated by `;` (`case 1 -> foo();`),
    // matching the Java/Groovy `case L -> expr;` arrow form. The `;`
    // is anonymous and absorbed here rather than by the switch block,
    // which has no statement-list of its own.
    switch_arrow_case: $ => seq(
      'case',
      caseValues($),
      '->',
      field('body', choice($._body_block, $._statement)),
      optional(';'),
    ),

    switch_default: $ => prec.left(seq(
      'default',
      choice(
        seq(':', statementList($)),
        seq('->', field('body', choice($._body_block, $._statement)), optional(';')),
      ),
    )),

    // §3.2 — `x as Type`. TYPE_TEST level, left-associative.
    // RHS is `_cast_type`, a `_type`-shaped sibling that swaps in
    // `_dotted_type` for `qualified_type` so the trailing
    // `.identifier` of `x as java.util.Map` extends the type
    // through every dot segment (instead of being claimed by
    // `field_access` at higher precedence than `cast_expression`).
    cast_expression: $ => prec.left(PREC.TYPE_TEST, seq(
      field('value', $._expression),
      alias($._as_keyword, 'as'),
      field('type', $._cast_type),
    )),

    // §3.2.2 — RHS type of `as`, `instanceof`, and `!instanceof`.
    // Same alternatives as `_type`, with `_dotted_type` in place of
    // `qualified_type`, also as the base of an array or generic type:
    // it shifts each dot at a precedence tighter than `field_access`,
    // which the repeat of `qualified_type` does not, so that
    // `x as a.b.C[]` and `x as java.util.List<String>` keep their type
    // past every dot.
    _cast_type: $ => choice(
      alias($.identifier, $.type_identifier),
      alias($._cast_array_type, $.array_type),
      alias($._dotted_type, $.qualified_type),
      alias($._cast_generic_type, $.generic_type),
    ),

    // The element is a rule of its own, not `_cast_type`: after a whole
    // `_cast_type` the `[` of a dimension would compete with the reduction
    // of the left-associative `cast_expression`, which precedence settles
    // for the reduction, so `x as T[]` would not parse. The element and
    // the whole type compete as two reductions instead, until the `]`.
    _cast_array_type: $ => seq(
      $._cast_array_element,
      alias($._subscript_open, '['),
      ']',
    ),

    _cast_array_element: $ => choice(
      alias($.identifier, $.type_identifier),
      alias($._cast_array_type, $.array_type),
      alias($._dotted_type, $.qualified_type),
      alias($._cast_generic_type, $.generic_type),
    ),

    _cast_generic_type: $ => seq(
      field('base', choice(
        alias($.identifier, $.type_identifier),
        alias($._dotted_type, $.qualified_type),
      )),
      field('arguments', alias($._cast_type_arguments, $.type_arguments)),
    ),

    // The `<` is the scanner's, emitted only where the arguments close:
    // where they do not, Groovy compares the cast with what follows
    // (`x as a.B<c`, `v as java.lang.Integer<max && min>v`), and the `<`
    // is the comparison's.
    _cast_type_arguments: $ => seq(
      alias($._cast_type_arguments_open, '<'),
      $._type_argument,
      repeat(seq(',', $._type_argument)),
      '>',
    ),

    // The first dot continues the type, at the precedence of the others.
    // `x as java` followed by field accesses was the other reading the
    // dot kept, and with type arguments its comparisons
    // (`(x as java).util.List < T > …`) took the next line as an operand
    // and forked again there, so a run of such lines took quadratic time.
    // Without that reading, a quoted name or `new` after a dot of the type
    // (`x as a."b"`, `x as a.new`) is an ERROR; Groovy reads a command
    // chain on a variable `as` there (docs/divergences-from-spec.md §5).
    _dotted_type: $ => weighted(DOTTED_CAST_TYPE_WEIGHT, prec(PREC.ACCESS + 1, seq(
      alias($.identifier, $.type_identifier),
      $._dotted_type_tail,
    ))),

    _dotted_type_tail: $ => prec.right(PREC.ACCESS + 1, seq(
      '.',
      alias($.identifier, $.type_identifier),
      optional($._dotted_type_tail),
    )),

    // §3.2 / §3.2.2 — `x in xs` and `x !in xs`. RELATIONAL level. Both
    // operators share one node kind; the `operator` field
    // distinguishes them. `!in` is an operator only before whitespace
    // or `[`, `(`, `{` (Apache's `NOT_IN` predicate), so that
    // `a !inXs` is not `!in` followed by `Xs`. The external scanner
    // makes that check (§6.12) and leaves the character where it is.
    // As a regex the token had to include it, which made
    // `a !in[1, 2]` an ERROR: the `[` was part of the operator.
    membership_expression: $ => {
      const ops = [
        alias($._in_keyword, 'in'),
        alias($._not_in, '!in'),
      ];
      return choice(...ops.map(op => prec.left(PREC.RELATIONAL, seq(
        field('element', $._expression),
        field('operator', op),
        field('collection', $._expression),
      ))));
    },

    // §3.2 / §3.2.2 — `x instanceof T` and `x !instanceof T`, at TYPE_TEST.
    // `!instanceof` is a single token whose match requires a
    // trailing whitespace (Apache `NOT_INSTANCEOF` predicate) so
    // `a !instanceofX` does not mis-tokenise as `!instanceof`
    // followed by identifier `X`.
    instanceof_expression: $ => {
      const ops = [
        'instanceof',
        token(prec(1, seq('!instanceof', /[ \t\r\n]/))),
      ];
      return choice(...ops.map(op => prec.left(PREC.TYPE_TEST, seq(
        field('value', $._expression),
        field('operator', op),
        field('type', $._cast_type),
      ))));
    },

    // §3.2 — `new Foo(args)` and the anonymous inner class
    // `new Foo(args) { members }`. Array creation is
    // `array_creation_expression` below. PREC.PRIMARY only labels the
    // §3.1 tier. The number decides nothing: the rule's one conflict
    // (below) is against itself, where it cancels out, and
    // `new Foo().bar` composes without any precedence.
    //
    // The optional body takes every `{` that follows the arguments.
    // Right associativity does that statically — the shift into
    // `class_body` beats reducing the bodiless form — so no conflict
    // is declared and the closure reading never forks.
    //
    // Apache is narrower. Its `creator` has no `nls` before the body,
    // so `new T()\n{ … }` is a closure called on the new object
    // (observed on Groovy 4.0.33). By the same grammar a same-line
    // `{` whose content cannot be class members should be one too
    // (`new T() { x -> x }`; not run through the compiler). Here both
    // are read as a body, usually with an ERROR in it (divergence 11).
    // Parsing them needs a declared conflict on this rule with
    // `prec.dynamic` on the body, and that form was not taken: a body
    // holding one member `class_body` cannot parse, or one still
    // being typed, would then fall back silently to the closure tree
    // that #68 reported.
    object_creation_expression: $ => prec.right(PREC.PRIMARY, seq(
      'new',
      field('type', choice($._created_type, alias($._diamond_type, $.generic_type))),
      field('arguments', $.argument_list),
      optional(field('body', $.class_body)),
    )),

    // Apache `createdName`: a `_type` without array dimensions, which
    // belong to the array creation. With `array_type` here, the `[`
    // after `new int` could open the type `int[]` or a dimension.
    _created_type: $ => choice(
      alias($.identifier, $.type_identifier),
      $.qualified_type,
      $.generic_type,
    ),

    // §3.2 — Apache's createdName permits diamond, but ordinary types
    // (including nested type arguments) do not. Keep the public shape.
    _diamond_type: $ => prec.right(PREC.ARRAY_TYPE, seq(
      field('base', choice(
        alias($.identifier, $.type_identifier),
        $.qualified_type,
      )),
      field('arguments', alias($._diamond_type_arguments, $.type_arguments)),
    )),

    _diamond_type_arguments: _ => seq(TYPE_ARGUMENTS_LT, '>'),

    // §3.2 — `new int[n][m]`, `new int[n][]` and `new int[] { … }`,
    // after Apache `creator`: sized dimensions and then empty ones, or
    // empty dimensions and an initializer. Right associativity makes a
    // same-line `[` after a dimension another dimension rather than a
    // subscript, so `new int[2][3]` is two-dimensional, as Apache's
    // greedy loops read it. A `[` on the next line is a list literal
    // (§6.9).
    //
    // The number of `[]` picks the initializer. In Groovy 3 and 4 an
    // element is an expression, so a `{` among the elements is a
    // closure: `new Closure[] { { it * 2 } }`. Groovy 5 (GROOVY-8551)
    // nests initializers. They are taken only after two or more `[]`,
    // where the Groovy 4 reading puts a closure in an element that must
    // itself be an array.
    array_creation_expression: $ => prec.right(PREC.PRIMARY, seq(
      'new',
      field('type', $._created_type),
      choice(
        seq(
          repeat1(field('dimensions', $.dimensions_expr)),
          optional(field('dimensions', $.dimensions)),
        ),
        seq(
          field('dimensions', alias($._one_dimension, $.dimensions)),
          field('value', alias($._flat_array_initializer, $.array_initializer)),
        ),
        seq(
          field('dimensions', alias($._many_dimensions, $.dimensions)),
          field('value', $.array_initializer),
        ),
      ),
    )),

    dimensions_expr: $ => seq(alias($._subscript_open, '['), $._expression, ']'),

    dimensions: $ => prec.right(repeat1(seq(alias($._subscript_open, '['), ']'))),

    _one_dimension: $ => seq(alias($._subscript_open, '['), ']'),

    _many_dimensions: $ => seq($._one_dimension, repeat1($._one_dimension)),

    // Groovy 3 and 4 `arrayInitializer`. Groovy 2 read these braces as
    // a closure.
    _flat_array_initializer: $ => arrayInitializer($._expression),

    // Groovy 5 `arrayInitializer`. A nested `{}` or `{ x }` could also
    // be a closure; `prec(1)` makes it an initializer, the alternative
    // Groovy 5 tries first.
    array_initializer: $ => prec(1, arrayInitializer(
      choice($._expression, $.array_initializer),
    )),

    // §5.15 — C-style cast `(Foo) x`. Distinct from
    // `parenthesized_expression` followed by something (resolved
    // by tree-sitter's GLR; declared as a conflict if needed).
    // PREC.CAST right-associative so `(Foo) -x` parses as
    // `(Foo)(-x)` and the type binds to the rightward expression. It is
    // above POWER, so `**` applies to the cast (`(int) a ** b`); a prefix
    // `-` in the operand still takes a power after it, so
    // `(int) -a ** b` is `(int) (-(a ** b))`, where Groovy casts `-a`
    // (docs/divergences-from-spec.md §32).
    //
    // Two weights rank the cast against the other reading of the same
    // tokens, a parenthesized expression followed by something. Both
    // follow Apache's `castParExpression castOperandExpression`.
    //
    // With the operand on the line of the `)`, the cast wins wherever
    // it parses, as the cast alternative comes first in Apache's
    // `expression`: `(a) + b`, `(a) [0]` and `(a) (b)` are casts.
    // `_cast_close` (§6.11) is the `)` that has something after it on
    // its line.
    //
    // With the operand on a later line, the cast loses to any other
    // complete reading. Apache's rule has no `nls` between the two
    // parts, so where a line break can end a statement, `x = (a)`
    // followed by `foo()` is two statements. Inside `(…)` and `[…]`
    // Groovy ignores line breaks and reads a cast. Rejecting the cast
    // outright would lose every such case: the scanner cannot tell
    // which brackets enclose it. Ranking it last keeps the ones where
    // nothing else parses, `foo((String)` followed by `x)`. It loses
    // the ones where the later line can also continue the expression:
    // `foo((a)` followed by `- b)` is a subtraction here and a cast
    // in Groovy (divergence 15, #99).
    //
    // Without the weights nothing ranked the two readings and the
    // stack order decided. Inside a closure or block, `a = (int) y`
    // followed by a line that starts with a literal lost the cast
    // once #83 added its forks.
    //
    // A `)` before `instanceof`, `!instanceof`, `!in`, or `as` or `in`
    // and what it takes, is `_relational_close`, which closes no cast:
    // Groovy reads no cast of a name `instanceof` there, nor, from
    // Groovy 6.0.0 on (GROOVY-10355), one of `as` or `in` (§6.11).
    parenthesized_type_cast: $ => choice(
      weighted(CAST_WEIGHT, prec.right(PREC.CAST, seq(
        statementParen($),
        field('type', $._type),
        choice(alias($._cast_close, ')'), alias($._cast_word_close, ')')),
        field('value', $._expression),
      ))),
      // The cast whose operand is on a later line, in two alternatives by
      // its type. A name or a dotted name may be an expression too, and the
      // scanner turns that cast down with `_relational_close` where its `)`
      // ends a line before a line that opens with `(` and no bracket holds
      // both lines (§6.11): that `(` opens the next statement, and the cast,
      // kept alive across it, was reduced again at every such line. An
      // array or generic type is no expression, and its cast is the only
      // reading of the parentheses: `_type_only_close`, which the scanner
      // never emits, is valid before its `)` and keeps that cast.
      weighted(-CAST_WEIGHT, prec.right(PREC.CAST, seq(
        statementParen($),
        field('type', $._cast_name_type),
        ')',
        field('value', $._expression),
      ))),
      weighted(-CAST_WEIGHT, prec.right(PREC.CAST, seq(
        statementParen($),
        field('type', choice($.array_type, $.generic_type)),
        choice(')', $._type_only_close),
        field('value', $._expression),
      ))),
    ),

    // The type of a cast whose operand is on a later line, where the
    // parentheses may hold an expression instead.
    _cast_name_type: $ => choice(
      alias($.identifier, $.type_identifier),
      $.qualified_type,
    ),

    // §4 — `_type`. Supports unqualified type identifiers,
    // qualified types (`java.util.List`), array types
    // (`int[]`, `String[][]`), and generic types (`List<String>`,
    // `Map<String, Integer>`). `generic_type` always carries explicit
    // angle brackets, so it never competes with comparison operators
    // in expression context (which only reach `_type` through a
    // separate parse stack).
    _type: $ => choice(
      alias($.identifier, $.type_identifier),
      $.array_type,
      $.qualified_type,
      $.generic_type,
      // §6.15 — never matches, as the scanner never emits `_type_start`.
      // It makes the token valid wherever a type may start.
      seq($._type_start, alias($.identifier, $.type_identifier)),
    ),

    // Near-twin of `_dotted_type` below; the two cannot be merged
    // without breaking either field-access (which needs
    // `qualified_type` to NOT outrank `field_access`) or
    // cast / instanceof RHS (which needs `_dotted_type` to outrank
    // `field_access`). Edits here should be mirrored in
    // `_dotted_type` / `_dotted_type_tail`.
    qualified_type: $ => prec.left(seq(
      alias($.identifier, $.type_identifier),
      repeat1(seq('.', alias($.identifier, $.type_identifier))),
    )),

    array_type: $ => prec(PREC.ARRAY_TYPE, seq($._type, alias($._subscript_open, '['), ']')),

    // §6.15 — an array type as a value (`String[].class`, `int[]::new`,
    // `@A(String[])`), which Groovy reads as a class (Apache's `primary`
    // or a dotted name, then an `indexPropertyArgs` with no index). The
    // scanner emits `_array_value_start` before the name only when `[]`
    // follows it, so that a name in an expression is reduced as before;
    // the parser cannot wait for the `]` without keeping every such name
    // apart from a plain expression. The precedence makes a further `[`
    // a dimension, not a subscript.
    _array_type_value: $ => prec(1, seq(
      choice(
        seq(
          $._array_value_start,
          choice(alias($.identifier, $.type_identifier), $.qualified_type),
        ),
        alias($._array_type_value, $.array_type),
      ),
      alias($._subscript_open, '['),
      ']',
    )),

    // §4 / §5.14 — Java-style generic type arguments. Allows nested
    // generics (`Map<String, List<Integer>>`) and bounded wildcards
    // (`? extends Foo`, `? super Bar`). The `<` only matches when the
    // angle bracket directly follows the base name with no intervening
    // whitespace — `a < b` still parses as a binary expression because of
    // the space before `<` — and is the scanner's where the arguments close
    // or run past its read, so that it is no comparison's
    // (`TYPE_ARGUMENTS_LT`).
    generic_type: $ => prec.right(PREC.ARRAY_TYPE, seq(
      field('base', choice(
        alias($.identifier, $.type_identifier),
        $.qualified_type,
      )),
      field('arguments', $.type_arguments),
    )),

    type_arguments: $ => seq(
      choice(TYPE_ARGUMENTS_LT, alias($._type_arguments_open, '<')),
      $._type_argument,
      repeat(seq(',', $._type_argument)),
      '>',
    ),

    _type_argument: $ => choice(
      $._type,
      $.wildcard,
    ),

    wildcard: $ => seq(
      '?',
      optional(seq(
        choice('extends', 'super'),
        $._type,
      )),
    ),

    // §3.1 ACCESS tier / §3.2 — seven distinct node kinds for the
    // dot family. All at PREC.ACCESS left-associative so chains like
    // `a.b.c` parse `(a.b).c`. Right operand is the property /
    // field / method name (an identifier; method_reference also
    // accepts the `new` keyword per §5.14). Each operator emits a
    // unique node kind so downstream tooling (`get_op_type()`) can
    // discriminate without tree-shape heuristics.

    field_access: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.',
      field('field', $._property_name),
    )),

    safe_navigation_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '?.',
      field('property', $._property_name),
    )),

    // Groovy 4+ — see SPECIFICATION.md §3.2. Unlike `?.`, which only
    // short-circuits when the immediate receiver is null, `??.`
    // propagates null through the whole chain.
    safe_chain_dot_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '??.',
      field('property', $._property_name),
    )),

    spread_dot_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '*.',
      field('property', $._property_name),
    )),

    method_pointer_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.&',
      field('method', $._property_name),
    )),

    direct_field_access_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.@',
      field('field', $._property_name),
    )),

    // Apache `pathElement` takes the `@` of a direct field access after
    // `?.`, `??.` and `*.` as well as after `.`.
    safe_direct_field_access_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '?.@',
      field('field', $._property_name),
    )),

    safe_chain_direct_field_access_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '??.@',
      field('field', $._property_name),
    )),

    spread_direct_field_access_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '*.@',
      field('field', $._property_name),
    )),

    // §5.2 — property / method names after a dot operator (or as a
    // method declaration name) can be either an identifier or a
    // *quoted identifier*: a string literal that escapes spaces,
    // keywords, or special chars. `map."key with spaces"` and
    // `def "abstract"() { … }` both rely on this.
    _property_name: $ => choice(
      reserved('property', $.identifier),
      alias($.string_literal, $.quoted_identifier),
    ),

    // §5.14 — method reference. Groovy 3+. The RHS is either an
    // identifier (`String::length`) or the `new` keyword
    // (`Foo::new`).
    method_reference_expression: $ => prec.left(PREC.ACCESS, seq(
      field('target', $._expression),
      '::',
      field('name', choice(reserved('property', $.identifier), 'new')),
    )),

    // §3.2 — postfix `++` / `--`. Distinct node kind from the
    // prefix form (`unary_update_expression`) per the spec table.
    // PREC.POSTFIX, left-associative.
    update_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('operand', $._expression),
      field('operator', choice('++', '--')),
    )),

    // §3.2 — prefix `++` / `--`. Apache groups these with UNARY_ADD
    // (level 3); we use PREC.UNARY to match.
    unary_update_expression: $ => prec.right(PREC.UNARY, seq(
      field('operator', choice('++', '--')),
      field('operand', $._expression),
    )),

    // §3.2 POSTFIX / §3.2.1 — `f(args)` and the subscript-yielding-
    // callable composition `arr[i](args)`. The callee slot is any
    // expression so subscript / future field_access / future
    // safe_navigation chains compose naturally. PREC.POSTFIX
    // matches subscript so chains like `obj[i](x)[j]` work.
    //
    // Trailing closure (§5.5's command_chain covers the bare-
    // identifier statement form; this covers the general expression
    // form used constantly on property-access chains: `list.find {
    // it > 5 }`, `list.inject(0) { acc, x -> acc + x }`). Groovy
    // treats a closure literal as the call's last argument whether
    // or not parens are present, so `argument_list` becomes optional
    // once a trailing closure is there to anchor the call.
    //
    // The args-and-closure alternative sits one precedence level
    // above plain POSTFIX so `f(x) { … }` reduces to a single
    // method_invocation node (arguments AND closure fields on the
    // same node) instead of GLR instead reaching the same span by
    // nesting a closure-only method_invocation around a plain call.
    //
    // The arguments take only the in-grammar `(`. A `(` that starts a
    // line where a statement may start is `_statement_paren` (§6.18),
    // which opens the next statement, as Apache's `arguments` has no `nls`
    // before it. A `{` on the next line still joins (`pathElement`'s
    // `nls closureOrLambdaExpression`).
    method_invocation: $ => choice(
      prec.left(PREC.POSTFIX + 1, seq(
        field('function', $._expression),
        field('arguments', $.argument_list),
        field('closure', $.closure),
      )),
      prec.left(PREC.POSTFIX, seq(
        field('function', $._expression),
        field('arguments', $.argument_list),
      )),
      prec.left(PREC.POSTFIX, seq(
        field('function', $._expression),
        field('closure', $.closure),
      )),
      // A statement that calls a name with only a closure and goes on with
      // a member access (`foo { }.bar()`), which `command_chain` would
      // take as a whole statement. The scanner emits `_closure_call_start`
      // before the name only where the closure closes on the name's line
      // and an access follows it (§6.24). An alternative that started with
      // the name itself would keep the state after every name in a value
      // from being a lone unit reduction.
      seq(
        $._closure_call_start,
        field('function', $.identifier),
        field('closure', $.closure),
      ),
    ),

    // §3.2 / §5.11 — argument list. Positional args are `_expression`;
    // named args (Groovy's `k: v` in call position) are `map_entry`
    // aliased to `named_argument` so downstream tools can tell them
    // apart from positional values.
    argument_list: $ => argumentList($._argument),

    _argument: $ => choice(
      enhancedExpression($),
      alias($.map_entry, $.named_argument),
      $.spread_arguments,
      // Apache `namedArg: … | MUL COLON nls expression`: `foo(*:m, x: 1)`.
      $.spread_map_entry,
    ),

    // §5.11 — `*expr` only inside an argument list, a list literal or a
    // subscript.
    // Distinct from a binary `*` because the LHS is absent.
    spread_arguments: $ => seq(
      '*',
      field('value', $._expression),
    ),

    // §3.1 POSTFIX / §3.2 — subscript and safe-subscript. The
    // distinction `arr?[k]` (safe) vs `arr[k]` (regular) is the
    // explicit ask of dekobon #247 for the `?[]` row. Both at
    // PREC.POSTFIX, left-associative — `a[b][c]` parses as
    // `(a[b])[c]`.
    // `[` must be on the object's line (see `_subscript_open`): a Grails
    // controller action that ends in `[model: value]` on its own line returns
    // a map; it isn't a subscript of the statement before it.
    subscript_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('object', $._expression),
      alias($._subscript_open, '['),
      $._indices,
      ']',
    )),

    // `?[` is one token. `a ? [b] : c` (with space) still parses
    // as ternary plus list because tree-sitter's lexer never
    // crosses a whitespace boundary to extend a multi-char token.
    // Unlike `[` (see `_subscript_open`), `?[` may still follow a
    // newline: no statement starts with `?[`, so there is nothing
    // to split off.
    safe_subscript_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('object', $._expression),
      '?[',
      $._indices,
      ']',
    )),

    // Apache `indexPropertyArgs: (SAFE_INDEX | LBRACK) expressionList[true]?
    // RBRACK`: several indices, any of them a spread (`y[1, *a]`), an
    // `index` field each. One rule for both subscripts shares the states of
    // the list: 6 more in all, where a list in each cost 20.
    _indices: $ => seq(
      field('index', $._index_element),
      repeat(seq(',', field('index', $._index_element))),
    ),

    _index_element: $ => choice($._expression, $.spread_arguments),

    // §3.1 level 2 / §3.2 — `**`. Distinct node kind so metric tools
    // count power separately from multiplicative ops. Left-associative,
    // as Apache's powerExprAlt has no `<assoc=right>`: `2 ** 3 ** 2` is
    // `(2 ** 3) ** 2`, 64 in Groovy. POWER binds tighter than UNARY
    // (+ -) but looser than UNARY_NOT (! ~) — see PREC table comments —
    // so `-2 ** 2` is `-(2 ** 2)` and `2 ** -2 ** 3` is
    // `2 ** -(2 ** 3)`.
    power_expression: $ => prec.left(PREC.POWER, seq(
      field('left', $._expression),
      '**',
      field('right', $._expression),
    )),

    // §3.1 level 13.5 / §3.2 — `==>`. Right-associative per Apache
    // (`<assoc=right>`), distinct node kind so downstream tools can
    // discriminate from `=>` (lambda arrow, not yet wired).
    logical_implication_expression: $ => prec.right(PREC.IMPLICATION, seq(
      field('left', $._expression),
      '==>',
      field('right', $._expression),
    )),

    // §3.1 level 15 / §3.2 — single `assignment_expression` node
    // covers every augmented form (`+=`, `-=`, `*=`, `/=`, `%=`,
    // `**=`, `<<=`, `>>=`, `>>>=`, `&=`, `^=`, `|=`) plus plain `=`
    // and Elvis-assign `?=`. The operator field distinguishes them.
    // Right-associative per Apache. ASSIGN is the loosest precedence
    // tier and binds looser than CONDITIONAL above it.
    assignment_expression: $ => {
      const ops = [
        '=',
        '+=', '-=', '*=', '/=', '%=', '**=',
        '<<=', '>>=', '>>>=',
        '&=', '^=', '|=',
        '?=',
      ];
      return choice(...ops.map(op => prec.right(PREC.ASSIGN, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', enhancedStatementExpression($)),
      ))));
    },

    // §4 — `(a, b) = expr`, Apache `#multipleAssignmentExprAlt`: two or
    // more names and `=` alone. Apache lists it in `expression`, so it
    // nests (`x = (a, b) = list`). The names are identifiers, not
    // `variable_declarator`s: nothing is declared, and locals.scm reads
    // every declarator's name as a definition.
    //
    // A parenthesized expression or a cast has no `,` at its top level, so
    // the first `,` after `( identifier` decides with no declared conflict.
    // That `,` is `_multi_assignment_comma` (§6.20), the scanner's, aliased
    // to `,`. With one name, `(a) = 1` stays an `assignment_expression`, as
    // in Apache.
    //
    // After `def f` or `final f` where a statement can start, a declared
    // conflict keeps a reading in which the `(` opens the next statement.
    // It parses `def f` followed by `(a, b) = list` on the next line, and it
    // also accepts `def f(a, b) = list`, which Groovy rejects (divergence
    // 3). The scanner turns down the first `,` of `def f(a, b) { }`, which
    // ends that reading there. Decided at the `{`, it read every untyped
    // parameter a second time: 17% more instructions for a head with two
    // and 49% with eight. A token of the `,` itself, rather than an empty
    // one before it, costs a multi-assignment no parse step of its own.
    multi_assignment_expression: $ => prec.right(PREC.ASSIGN, seq(
      statementParen($),
      field('name', $.identifier),
      alias($._multi_assignment_comma, ','),
      field('name', $.identifier),
      repeat(seq(',', field('name', $.identifier))),
      ')',
      '=',
      field('value', statementExpression($)),
    )),

    // §3.1 level 14 / §3.2 — ternary and Elvis share the CONDITIONAL
    // precedence level and are both right-associative. Apache groups
    // them in one `conditionalExprAlt` so `a ? b ?: c : d` parses as
    // `a ? (b ?: c) : d` because the middle slot recursively re-enters
    // the same precedence level. We replicate this with two distinct
    // rules at PREC.CONDITIONAL; right-associativity + recursion via
    // `$._expression` give the same result.
    //
    // elvis_expression is the headline ask of
    // dekobon/big-code-analysis #246: downstream tools need to
    // discriminate elvis from ternary by node kind alone.
    ternary_expression: $ => prec.right(PREC.CONDITIONAL, seq(
      field('condition', $._expression),
      '?',
      field('consequence', $._expression),
      ':',
      field('alternative', $._expression),
    )),

    elvis_expression: $ => prec.right(PREC.CONDITIONAL, seq(
      field('value', $._expression),
      '?:',
      field('default', $._expression),
    )),

    // §3.2 — distinct-node ops that share precedence with existing
    // binary_expression tiers but emit unique node kinds per the
    // operator-coverage matrix. The integration target (`get_op_type()`
    // in dekobon/big-code-analysis) needs to discriminate these by
    // node kind alone, not by tree-shape heuristics.

    // §3.2 range_expression — inclusive `..`, exclusive `..<` (right),
    // exclusive `<..` (left), exclusive `<..<` (both). SHIFT_OR_RANGE
    // precedence level, left-associative.
    range_expression: $ => {
      const ranges = ['..', '..<', '<..', '<..<'];
      return choice(...ranges.map(op => prec.left(PREC.SHIFT_OR_RANGE, seq(
        field('start', $._expression),
        field('operator', op),
        field('end', $._expression),
      ))));
    },

    // §3.2 identity_expression — `===` (identity) and `!==`
    // (non-identity), EQUALITY level. Per Apache, these live in
    // the equality alternative.
    identity_expression: $ => {
      const ops = ['===', '!=='];
      return choice(...ops.map(op => prec.left(PREC.EQUALITY, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    // §3.2 spaceship_expression — `<=>`, EQUALITY level. Three-way
    // comparison; no `operator` field since there is only one form.
    spaceship_expression: $ => prec.left(PREC.EQUALITY, seq(
      field('left', $._expression),
      '<=>',
      field('right', $._expression),
    )),

    // §3.2 regex_find_expression — `=~`, REGEX level, looser than `==`
    // and tighter than `&`, as Apache's regexExprAlt follows
    // equalityExprAlt. Matches a pattern at any position. Fields are
    // `subject` / `pattern` so downstream tools can tell the
    // regex-bearing operand from the input expression.
    regex_find_expression: $ => prec.left(PREC.REGEX, seq(
      field('subject', $._expression),
      '=~',
      field('pattern', $._expression),
    )),

    // §3.2 regex_match_expression — `==~`, REGEX level. Anchored
    // whole-string match. Same field shape as regex_find_expression.
    regex_match_expression: $ => prec.left(PREC.REGEX, seq(
      field('subject', $._expression),
      '==~',
      field('pattern', $._expression),
    )),

    // §3.2 — single binary_expression node with an `operator` field
    // for every Java-shaped binary operator (per §3.2 table row 1).
    // Apache groups arithmetic operators across two precedence
    // levels (MUL: * / %, ADD: + -); we keep them on one rule with
    // distinct precedence numbers so the *node kind* stays uniform
    // while the precedence ladder matches Apache exactly.
    //
    // Currently covers ADDITIVE and MULTIPLICATIVE. Shift/range,
    // relational, equality, bitwise, and logical levels join this
    // rule in later iterations — each is a single line addition.
    binary_expression: $ => {
      /* eslint-disable no-multi-spaces --
       * Aligned columns make the precedence ladder readable.
       */
      const ops = [
        // ADDITIVE — §3.1 level 5
        ['+',   PREC.ADDITIVE],
        ['-',   PREC.ADDITIVE],
        // MULTIPLICATIVE — §3.1 level 4
        ['*',   PREC.MULTIPLICATIVE],
        ['/',   PREC.MULTIPLICATIVE],
        ['%',   PREC.MULTIPLICATIVE],
        // SHIFT — §3.1 level 6. Ranges (`..`, `..<`, etc.) share
        // this precedence level but use distinct node kinds, so
        // they live in their own rule.
        ['<<',  PREC.SHIFT_OR_RANGE],
        ['>>',  PREC.SHIFT_OR_RANGE],
        ['>>>', PREC.SHIFT_OR_RANGE],
        // RELATIONAL — §3.1 level 7. `in` and `!in` share this level
        // but emit a distinct node kind (`membership_expression`), so
        // they land in a rule of their own. `instanceof`,
        // `!instanceof` and `as` bind tighter (TYPE_TEST, the first
        // alternative of Apache's level 7) and have rules of their
        // own too (`instanceof_expression`, `cast_expression`).
        ['<',   PREC.RELATIONAL],
        ['<=',  PREC.RELATIONAL],
        ['>',   PREC.RELATIONAL],
        ['>=',  PREC.RELATIONAL],
        // EQUALITY — §3.1 level 8. `===`, `!==` and `<=>` share this
        // level but emit distinct node kinds. `=~` and `==~` bind
        // looser (REGEX, Apache's regexExprAlt after equalityExprAlt).
        ['==',  PREC.EQUALITY],
        ['!=',  PREC.EQUALITY],
        // BITWISE — §3.1 levels 9, 10, 11 (each tighter than the
        // next: & < ^ < | by precedence).
        ['&',   PREC.BITWISE_AND],
        ['^',   PREC.BITWISE_XOR],
        ['|',   PREC.BITWISE_OR],
        // LOGICAL — §3.1 levels 12, 13 (&& tighter than ||).
        ['&&',  PREC.LOGICAL_AND],
        ['||',  PREC.LOGICAL_OR],
      ];
      /* eslint-enable no-multi-spaces */
      return choice(...ops.map(([op, p]) => prec.left(p, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    // §5.8 — closure expression. `{ -> }` is the explicit no-parameter
    // form; `{ a, b -> body }` has identifier parameters;
    // `{ String a, int b -> body }` has typed parameters; `{ body }`
    // has no `->` and an implicit `it` parameter (we do NOT emit a
    // synthetic `it` per §5.8). A parameter takes what a method
    // parameter does (Apache's closure uses `formalParameterList`):
    // annotations, `final`, then `def`, `var` or a type (varargs
    // included), and a default; both rules are `parameter()`. The
    // body may open with the same words, so the `->` decides between
    // the two readings.
    closure: $ => seq(
      '{',
      optional($.closure_parameters),
      statementList($),
      '}',
    ),

    closure_parameters: $ => seq(
      optional(choice(
        seq(
          $.closure_parameter,
          repeat(seq(',', $.closure_parameter)),
        ),
        // §6.21 — never matches, as the scanner never emits
        // `_closure_parameters_start`. It makes the token valid where
        // closure parameters may start, the one place `String a = b -> c`
        // may be a parameter rather than a declaration.
        $._closure_parameters_start,
      )),
      '->',
    ),

    // The second alternative never matches, like the one of
    // `closure_parameters`: it keeps `_closure_parameters_start` valid
    // after a parameter's annotations, where the scanner decides the type
    // of `{ @A String a = 1 -> }` (§6.21). The token of
    // `closure_parameters` is valid only before them, so the scanner
    // opened a declaration after them. In `closure_parameters` the
    // alternative would cost half the states (36 against 70) and list
    // `annotation` among that node's children in `node-types.json`; the
    // name after the token keeps the `name` field required there.
    closure_parameter: $ => choice(
      parameter($, choice($._expression, $._bare_lambda_name)),
      seq(
        repeat1($.annotation),
        $._closure_parameters_start,
        field('name', $.identifier),
      ),
    ),

    // §5.16 — a Java-style lambda with parentheses, Apache's
    // `lambdaExpression`: `(a, b) -> a + b`, `() -> { }`,
    // `(int a, String b = 'x') -> …`. Apache lists it in `primary`, so it is
    // an expression wherever one is: an operand, a list element, the value
    // of `return`, a cast's operand (`(Runnable) () -> { }`). The parameters
    // are a method's (`formal_parameters`). `_lambda_start` (§6.17) is the
    // scanner's decision that the parentheses hold parameters and `->`
    // follows them; without it the parser could tell `(a) ->` from `(a)` only
    // at the `->`.
    //
    // The body takes all it can, as Apache's greedy `lambdaBody` does:
    // `(a) -> a + 1 as T` is a lambda whose body is the cast. The weight is
    // the body's (`lambdaBody`).
    lambda_expression: $ => prec.right(PREC.LAMBDA, seq(
      $._lambda_start,
      field('parameters', $.formal_parameters),
      '->',
      lambdaBody($, false),
    )),

    // A lambda whose one parameter has no parentheses (`x -> x + 1`),
    // Apache's `standardLambdaExpression` with a `variableDeclaratorId`,
    // aliased to `lambda_expression`. It stands only where Apache takes it
    // (see `enhancedStatementExpression`). `_bare_lambda_start` (§6.17) is
    // the scanner's decision that `->` follows the name. Decided by the
    // `->` after the name, every name in an argument or a value cost a
    // reduction of its own: tree-sitter skips the step from a name to an
    // expression only in a state that has no other action, and a call with
    // names for arguments took a fifth more reductions.
    _bare_lambda_expression: $ => prec.right(PREC.LAMBDA, seq(
      $._bare_lambda_start,
      field('parameters', $.identifier),
      '->',
      lambdaBody($, true),
    )),

    // The name of a closure parameter's default that `_bare_lambda_start`
    // opens. At the start of a closure the default and the value of a
    // declaration or an assignment share their tokens (`{ a = b -> c }`,
    // `{ String a = b -> c }`), and the scanner reads only the name and the
    // `->`. Where a value is a closure, Groovy reads the parameter; where a
    // block is possible too, the body of `if` (`if (x) { a = b -> c }`) or a
    // `{` that opens a statement, it reads a block that assigns a lambda. A
    // static decision would hold in both, as the states are shared, so the
    // precedence ties with the lambda's and the
    // `[_bare_lambda_name, _bare_lambda_expression]` conflict keeps both
    // readings for the weights to rank.
    _bare_lambda_name: $ => prec(PREC.LAMBDA, seq($._bare_lambda_start, $.identifier)),

    // A block as the body of `else`, a loop or an arrow case, ranked above a
    // closure (BODY_BLOCK_WEIGHT). A rule of its own costs every such body a
    // reduction; the weight inline in the six loop and case rules took about
    // 50 more states, and inline in `else` it was lost, as a production takes
    // the largest of the weights in it, not their sum, and the `if` before it
    // has one. Aliased to itself so that the reduction, and the weight, is
    // kept. `_body_block_end`, which the scanner never emits, is valid after
    // the `}`: there NOT_IN is valid too, as the braces may be a closure,
    // and the token tells the scanner that a `(` there may open a lambda
    // statement, as it does where a reparse that reused the body lexes the
    // `(` at a statement's start (§6.17).
    _body_block: $ => weighted(BODY_BLOCK_WEIGHT, seq(
      alias($.block, $.block),
      optional($._body_block_end),
    )),

    // The consequence of `if`, as a rule of its own: its two readings, a
    // block and a closure (`{ a = (b) -> c }`), must reduce to one symbol at
    // its `}`, where the runtime merges them and the weight decides. Inline in
    // `if_statement` they were ranked only when that statement was reduced,
    // which in an `else if` chain is at the chain's end: every link kept both
    // readings alive, the parse took time far above linear in the length of
    // the chain, and past the runtime's limit on stack versions the block
    // reading of a link was dropped for the closure.
    _if_consequence: $ => choice($._body_block, $._statement),

    // A parenthesized expression after `_lambda_start`, aliased to
    // `parenthesized_expression`. The scanner reads only the parentheses
    // and the `->`, and an expression may be followed by `->`: a closure
    // parameter's default (`{ a = (b) -> c }`) and a case value
    // (`case (A) -> 1`). Groovy reads both so. The `[formal_parameter,
    // _expression]` conflict keeps the lambda reading too, until the token
    // after the `->` or the end of the case ends it; where both survive, the
    // lambda's weight ranks it last. Elsewhere this reading ends at the `->`,
    // which nothing else takes after an expression, two tokens after the
    // `(`, but its stack version first reduces every lambda still open before
    // it, as the parentheses may be the whole body of each: a nest of lambdas
    // whose bodies are expressions, `(a) -> (a) -> … a`, takes reductions
    // quadratic in its depth (SPECIFICATION.md §6.17).
    _lambda_parenthesized_expression: $ => seq(
      $._lambda_start,
      '(',
      statementExpression($),
      ')',
    ),

    // §3.2 PRIMARY / §5.13 — list literal with trailing-comma support.
    // Empty list is `[]`; the empty map literal is the distinct `[:]`
    // form (see map_literal). After the first element, a `:` or its
    // absence tells a list from a map within one token of lookahead,
    // so no conflict entry is needed.
    // Either `[` token opens a literal, so a same-line `[` resolves as
    // it did with one token: `if (a) { } [1].each { }` is still a block
    // followed by a list, not a subscripted closure.
    list_literal: $ => seq(
      choice('[', alias($._subscript_open, '[')),
      optional(seq(
        $._list_element,
        repeat(seq(',', $._list_element)),
        optional(','),
      )),
      ']',
    ),

    // Apache `list: LBRACK expressionList[true]? COMMA? RBRACK`: an element
    // may be a spread (`[*a, 1]`), the `spread_arguments` of a call.
    // Inlined (see `inline` above).
    _list_element: $ => choice($._expression, $.spread_arguments),

    // §3.2 PRIMARY / §5.3 — map literal. `[:]` is the empty map;
    // anything else is one-or-more `key: value` entries. The
    // distinction between map_entry and a labelled statement is
    // contextual — labels are statement-start-only (see §5.3 and
    // the LABEL_COLON scanner branch), so inside `[ … ]` a `:` after
    // an identifier is always a map key separator.
    map_literal: $ => {
      const entry = choice($.map_entry, $.spread_map_entry);
      return seq(
        choice('[', alias($._subscript_open, '[')),
        choice(
          ':',
          seq(
            entry,
            repeat(seq(',', entry)),
            optional(','),
          ),
        ),
        ']',
      );
    },

    map_entry: $ => seq(
      field('key', choice($._expression, alias('case', $.identifier))),
      ':',
      field('value', enhancedExpression($)),
    ),

    // §5.11 — `*: expr` inside a map literal or an argument list. `*:` is
    // a single anonymous token (longest-match keeps it intact); a bare `*`
    // followed by a `:` colon outside these contexts would have to be
    // syntactically impossible by the surrounding rules.
    spread_map_entry: $ => seq(
      '*:',
      field('value', $._expression),
    ),

    // §3.2 PRIMARY — `( expr )`. Distinct from `parenthesized_type_cast`
    // (§5.15); the parser disambiguates by what's inside the parens
    // and after them. `_cast_close` is the `)` that has something
    // after it on its line (§6.11). It closes this rule too, so that
    // `(a) * b` and `((a))` stay open to both readings. Only this rule
    // takes `_relational_close`, the `)` before a relational operator that
    // Groovy applies to the parentheses (`(x) instanceof Foo`). The content may
    // be a parenthesis-free call (`(sh script: 'x').trim()`, §5.5 item 1).
    //
    // Before a name on the line, `_cast_word_close`, the cast outranks this
    // rule by more than the typed member that the name could start. Members
    // need no separator here, so `def k = (Foo) x` then `def m() { }` was
    // also the field `k = (Foo)`, a field `x def` and a constructor `m`,
    // which weighed as much as the cast and the method and won the tie, and
    // `public C() { }` after it the field `x public` and a constructor,
    // which weighed more. Groovy needs a separator between members and casts
    // the name.
    parenthesized_expression: $ => seq(
      statementParen($),
      enhancedStatementExpression($),
      choice(
        ')',
        alias($._cast_close, ')'),
        weighted(-TYPED_FIELD_WEIGHT, alias($._cast_word_close, ')')),
        alias($._relational_close, ')'),
      ),
    ),

    // §3.2 — single `unary_expression` node kind but two precedence
    // alternatives, matching Apache §3.1:
    //   UNARY_NOT (`!` `~`)  — level 18, tighter than POWER (17),
    //                          so `~2 ** 3` parses as `(~2) ** 3`.
    //   UNARY     (`+` `-`)  — level 16, looser than POWER (17),
    //                          so `-2 ** 3` parses as `-(2 ** 3)`.
    // Both alternatives produce the same node kind, so downstream
    // tools see one consistent shape and discriminate by operator.
    unary_expression: $ => choice(
      prec.right(PREC.UNARY_NOT, seq(
        field('operator', choice('!', '~')),
        field('operand', $._expression),
      )),
      prec.right(PREC.UNARY, seq(
        field('operator', choice('+', '-')),
        field('operand', $._expression),
      )),
    ),

    // §3.2 PRIMARY — boolean and null are keywords in Groovy. The
    // `word: $.identifier` directive above lets tree-sitter resolve
    // the keyword-vs-identifier ambiguity automatically: a literal
    // 'true' / 'false' / 'null' that matches the identifier regex
    // is reclassified as the keyword token when one of these rules
    // is the only legal next reduction.
    boolean_literal: _ => choice('true', 'false'),

    null_literal: _ => 'null',

    // §5.7 — string flavours. Single and triple-single never
    // interpolate. Double-quoted and triple-double interpolate
    // `$identifier(.identifier)*` and `${expression}` as structured
    // children, so consumers can walk into the GString's expression
    // segments. Slashy bodies and dollar-slashy bodies remain flat
    // tokens (their internal `$` is rare in practice and the
    // scanner cannot interrupt a slashy mid-body without per-flavor
    // state).
    string_literal: $ => choice(
      $._single_quoted_string,
      $._triple_single_quoted_string,
      $._double_quoted_string,
      $._triple_double_quoted_string,
      $._dollar_slashy_string,
      $._slashy_string,
    ),

    // §5.7 — slashy regex `/.../`. The opening `/` is emitted by
    // the external scanner (after the context guard from §5.4 / §6.2
    // and a lookahead that confirms a closing `/` exists on the same
    // line). The body is parsed as `gstringPart` so `$identifier` and
    // `${expr}` segments expose the same structured children as
    // double-quoted GStrings. The closing `/` is `token.immediate`
    // so it cannot skip whitespace between the body and the closer.
    _slashy_string: $ => seq(
      $._slashy_string_start,
      repeat(gstringPart($, $._slashy_text)),
      token.immediate('/'),
    ),

    // A `\` takes the character after it, and a line break whole, `\r\n`
    // too, as Groovy's LineEscape does (`'a\` then `b'` on the next line is
    // `'ab'`): without the pair a CRLF file left the `\n` after the
    // escaped `\r` to end the string.
    _slashy_text: _ => token.immediate(repeat1(choice(
      /[^\/\\$]/,
      /\\(\r\n|[\s\S])/,
    ))),

    _single_quoted_string: _ => token(seq(
      '\'',
      repeat(choice(
        /[^'\\\n\r]/,
        /\\(\r\n|[\s\S])/,
      )),
      '\'',
    )),

    _triple_single_quoted_string: _ => token(seq(
      '\'\'\'',
      repeat(choice(
        /[^'\\]/,
        /\\[\s\S]/,
        /'[^']/,
        /''[^']/,
      )),
      '\'\'\'',
    )),

    // §5.7 / §6.3 — double-quoted GString. The opening `"` is
    // tokenised separately from the body so that `$identifier` and
    // `${expr}` segments parse as proper child nodes. The text
    // fragment stops before any `$`, so `$identifier` and `${expr}`
    // get a chance to match; literal `$` (followed by `"` or a
    // digit etc.) is recovered as `$` plus the fallback shape.
    _double_quoted_string: $ => seq(
      '"',
      repeat(gstringPart($, $._gstring_text)),
      token.immediate('"'),
    ),

    _gstring_text: _ => token.immediate(repeat1(choice(
      /[^"\\$\n\r]/,
      /\\(\r\n|[\s\S])/,
    ))),

    // Bare `$` inside a GString body that does not start a valid
    // interpolation (`$"`, `$5`, `$$`, `$ `) is matched as a one-
    // char literal fragment. This rule must have lower precedence
    // than `_gstring_dollar_path` so `$name` still wins.
    _gstring_literal_dollar: _ => token.immediate(prec(-1, /\$/)),

    // §5.7 — triple-double GString. Like `_double_quoted_string`
    // but newlines and embedded `"` / `""` are part of the body.
    //
    // One or two quotes are text when an ordinary character or an
    // escape follows them. Before `$` they are lexed one at a time, as
    // fragments of their own: the text token must not take a `$` that
    // starts an interpolation (`"""say "$name"""`), and a regex
    // cannot look ahead. The closer is longer than one quote, so it
    // still wins at `"""`.
    _triple_double_quoted_string: $ => seq(
      '"""',
      repeat(choice(
        gstringPart($, $._triple_gstring_text),
        alias(token.immediate('"'), $.string_fragment),
      )),
      token.immediate('"""'),
    ),

    _triple_gstring_text: _ => token.immediate(repeat1(choice(
      /[^"\\$]/,
      /\\[\s\S]/,
      /"(\\[\s\S]|[^"\\$])/,
      /""(\\[\s\S]|[^"\\$])/,
    ))),

    // §5.7 — `$identifier(.identifier)*` interpolation. The whole
    // `$identifier...` shape is matched as a single immediate token
    // so the `$` only consumes when followed by an identifier start;
    // `_gstring_literal_dollar` handles the lone-`$` fallback above.
    gstring_dollar_interpolation: $ => seq(
      field('value', alias(
        $._gstring_dollar_path,
        $.identifier,
      )),
    ),

    _gstring_dollar_path: _ => token.immediate(new RustRegex(
      String.raw`\$${identifierPattern(false)}(?:\.${identifierPattern(false)})*`,
    )),

    // §5.7 / §6.3 — `${expression}` interpolation. Groovy reads the braces
    // as a closure whose one statement is the value, so it may be a
    // parenthesis-free call (`"${sh script: 'x'}"`, §5.5 item 1).
    gstring_brace_interpolation: $ => seq(
      $._gstring_brace_open,
      field('value', statementExpression($)),
      '}',
    ),

    // Apache's `gstringValue` is a `closure`, so `"${-> x}"` is lazy: the
    // `->` of a closure without parameters, a `closure_parameters` leaf.
    // A rule of its own, so that both openers reach one state before the
    // value: `->` inline cost 20 states, and parameters before it 27.
    _gstring_brace_open: $ => seq(
      token.immediate('${'),
      optional(alias('->', $.closure_parameters)),
    ),

    // §5.7 — `$/...$/` dollar-slashy strings. The body is parsed
    // through `gstringPart` so `$identifier` and `${expr}` segments
    // expose structured children. `_dollar_slashy_text` consumes
    // every literal-text alternative: `/X` where X is not `$`,
    // `$$` (literal `$`), `$/` (literal `/`), and any other
    // non-`/` non-`$` char. The closer `/$` is `token.immediate`
    // so it always wins over text when `/` is followed by `$`.
    //
    // A run of slashes is text only up to an ordinary character
    // (`a//b`). The text token must not take a `/` that the closer
    // needs, and a regex cannot look ahead, so the slashes of a run
    // that ends at `$` are lexed one at a time: for `//$` the single
    // `/` matches first, then `/$` is the longer match and closes
    // the string. Each such slash is its own `string_fragment`.
    _dollar_slashy_string: $ => seq(
      '$/',
      repeat(choice(
        gstringPart($, $._dollar_slashy_text),
        alias(token.immediate('/'), $.string_fragment),
      )),
      token.immediate('/$'),
    ),

    _dollar_slashy_text: _ => token.immediate(repeat1(choice(
      /[^/$]/,
      /\/+[^/$]/,
      /\$\$/,
      /\$\//,
    ))),

    // §5.1 — hex (0x…), binary (0b…), decimal int / float / scientific,
    // each with optional type suffix from { G g L l I i D d F f }.
    // Underscores between digits are accepted permissively (the spec
    // forbids leading/trailing `_`, but that is a semantic check,
    // not a grammar one — see §1.2).
    number_literal: _ => token(choice(
      /0[xX][0-9a-fA-F_]+[GgLlIi]?/,
      /0[bB][01_]+[GgLlIi]?/,
      /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?[GgFfDd]?/,
      /[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*[GgFfDd]?/,
      /[0-9][0-9_]*[GgLlIiDdFf]?/,
    )),

    // Groovy's lexer takes any Java letter, not only ASCII; see
    // `identifierPattern` above for the classes.
    identifier: _ => new RustRegex(identifierPattern(true)),
  },
});
