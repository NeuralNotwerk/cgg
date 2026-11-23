// Hand-written external scanner for tree-sitter-groovy.
//
// Handles §6.2 slashy-string-start, §6.6 line/block/groovydoc
// comments, and the contextual `record` (§6.7), `trait` and `sealed`
// (§6.13) keywords. The first two start with `/`; combining their
// dispatch keeps the in-grammar lexer from partially consuming the
// leading `/`. The keywords are look-ahead decisions the grammar
// cannot express: each word is a keyword only when the declaration it
// opens follows it. §6.16 does the same for `var`, `yield`, `in` and
// `as`, each a keyword only before what it declares, takes or opens,
// and for such a word as a named argument's key (CONTEXTUAL_KEY).
// Also handles §6.8
// COMMAND_NAMED_KEY, an empty token before the key of a named
// argument in a parenthesis-free call; §6.14 COMMAND_VALUE_START, an
// empty token before the receiver of such a call that is a value,
// COMMAND_PATH_START, one between the first name of a dotted receiver
// and its first member access, and COMMAND_STATEMENT_START, one before
// a statement's one-name receiver whose first argument is a `new`, an
// array type, such a call in parentheses, a name or a literal that the
// argument goes on past;
// §6.9 SUBSCRIPT_OPEN, a `[` on
// the same line as the token before it; §6.18 STATEMENT_PAREN, a `(`
// that starts a line after an expression where a statement may start
// and the parentheses can open one; §6.15 ARRAY_VALUE_START, an
// empty token before an array type that is a value (`String[].class`);
// §6.11 CAST_CLOSE, a `)` with
// something after it on its line, CAST_WORD_CLOSE, one before a name on its
// line, and RELATIONAL_CLOSE, one before a
// relational operator that Groovy applies to the parentheses (`(x)
// instanceof Foo`) or one that ends its line before a line that opens with
// `(` where no bracket holds both lines, neither of which closes a cast;
// §3.2.2 CAST_TYPE_ARGUMENTS_OPEN, the `<` of a cast
// type's type arguments where they close, and TYPE_ARGUMENTS_OPEN, that of
// another generic type's where they close or run past the read; §6.12
// NOT_IN, the `!in`
// operator; §6.17 LAMBDA_START and BARE_LAMBDA_START, empty tokens
// before a lambda's parameters in parentheses and before its one name
// without them, when `->` follows; and §6.19 LAMBDA_BODY_END, an empty
// token after the `}` of a lambda's brace body that no expression
// continues, and LAMBDA_BODY_REREAD, an empty token no rule takes, which
// hands such a `}` back to the parser when it reused the body whole;
// §6.20 MULTI_ASSIGNMENT_COMMA, the first `,` of `(a, b) = …`, emitted
// unless the parentheses go on as a method's parameters; §6.21
// DECLARATION_START, an empty token before the type of a statement's typed
// local declaration (`String s`, `int a, b`, `String s = 'a'`), which the
// statement's first name decides with COMMAND_STATEMENT_START; §6.22
// STATEMENT_BLOCK_END, an empty token after the `}` of a block that opens a
// statement, decided as LAMBDA_BODY_END is where a statement may follow;
// and §6.23 JUMP_STATEMENT_END, an empty token after `return`, `break` or
// `continue` where a line break follows on the keyword's line.
// UNTERMINATED_COMMENT (§6.6) is an empty token at the
// first `/*` that has no `*/`, and carries the scanner's state; §6.19
// emits it after such a `}` as well, and §6.23 after `return`, `break` or
// `continue` before a block comment, to carry a decision past the token
// that follows.
// STRING_BODY (§6.10), TYPE_START (§6.15), CLOSURE_PARAMETERS_START
// (§6.21), TYPE_ONLY_CLOSE (§6.11) and BODY_BLOCK_END (§6.17) are never
// emitted; they are valid only inside a string body, where the scanner
// emits nothing, where a type may start, where closure parameters may
// start, before the `)` of a cast of an array or generic type whose operand
// is on a later line and after the `}` of the block that is the body of
// `if`, `else`, a loop or an arrow case. UNREACHABLE is never emitted and
// never read: it ends the grammar's error production for a `case` label
// outside a switch (§5.2). ERROR_SENTINEL is never emitted either;
// the parser marks it valid only during error recovery, where
// COMMAND_NAMED_KEY, CONTEXTUAL_KEY, COMMAND_VALUE_START, COMMAND_PATH_START,
// COMMAND_STATEMENT_START, SUBSCRIPT_OPEN, STATEMENT_PAREN,
// ARRAY_VALUE_START, CAST_CLOSE, CAST_WORD_CLOSE, RELATIONAL_CLOSE,
// CAST_TYPE_ARGUMENTS_OPEN, TYPE_ARGUMENTS_OPEN, NOT_IN, LAMBDA_START, BARE_LAMBDA_START,
// LAMBDA_BODY_END, LAMBDA_BODY_REREAD, MULTI_ASSIGNMENT_COMMA, DECLARATION_START, STATEMENT_BLOCK_END,
// JUMP_STATEMENT_END and the STRING_BODY check are suppressed. The keywords
// `record`, `trait`, `sealed`, `var`, `yield`, `in` and `as` are still
// decided there, by the same look-aheads, as are the comments,
// UNTERMINATED_COMMENT and SLASHY_STRING_START. The order in which the scan
// function tries the decisions is described above it.
//
// Four bits of state: `Scanner.no_comment_close_ahead` (§6.6),
// `Scanner.lambda_body_continues` (§6.19), `Scanner.jump_continues`
// (§6.23) and `Scanner.slashy_after_comment_newline` (§6.2). Nothing else
// is kept between calls.
//
// Known inconsistencies, kept as they are:
//   - Form feeds and vertical tabs. The grammar's extras skip them, and so
//     do the look-aheads of §6.20 and §6.21 (`skip_line_extras`,
//     `skip_extras`, `skip_multi_assignment_trivia`), but the others read
//     them as no trivia: `var\fx = 1` is a typed local of type `var`,
//     `x = y as\fFoo` three statements and `foo a\f: 1` an `ERROR`, where a
//     space makes the keyword, the cast and the named argument. Groovy's
//     lexer rejects both characters outside strings and comments, so no
//     input that Groovy accepts reads differently.
//   - Duplicated helpers. Look-aheads read the same lexical units with
//     helpers of their own: trivia (`skip_same_line_space`,
//     `skip_header_trivia`, `skip_argument_trivia`, `skip_same_line_trivia`,
//     `skip_line_extras`, `skip_extras`, `skip_multi_assignment_trivia`),
//     tokens with brackets counted (`parameters_follow`, `skip_lambda_tokens`,
//     `parentheses_open_statement`, `read_parentheses_extent`,
//     `next_line_paren_ends_cast`), strings
//     (`skip_string`, `skip_dollar_slashy_string`, `method_name_follows`)
//     and block comments
//     (`skip_comment_body`, and loops in `skip_header_trivia` and the scan
//     function). Each reads what its look-ahead was measured with; merging
//     them would change what some take for trivia, the form feeds above
//     among them, and needs every look-ahead's trees compared again.

#include "tree_sitter/alloc.h"
#include "tree_sitter/parser.h"

#include <string.h>

enum TokenType {
    SLASHY_STRING_START,
    LINE_COMMENT,
    BLOCK_COMMENT,
    GROOVYDOC_COMMENT,
    RECORD_KEYWORD,
    TRAIT_KEYWORD,
    SEALED_KEYWORD,
    VAR_KEYWORD,
    VAR_NAMES_KEYWORD,
    YIELD_KEYWORD,
    IN_KEYWORD,
    AS_KEYWORD,
    COMMAND_NAMED_KEY,
    CONTEXTUAL_KEY,
    COMMAND_VALUE_START,
    COMMAND_PATH_START,
    COMMAND_STATEMENT_START,
    SUBSCRIPT_OPEN,
    ARRAY_VALUE_START,
    CAST_CLOSE,
    NOT_IN,
    LAMBDA_START,
    BARE_LAMBDA_START,
    UNTERMINATED_COMMENT,
    STRING_BODY,
    TYPE_START,
    STATEMENT_PAREN,
    LAMBDA_BODY_END,
    LAMBDA_BODY_REREAD,
    MULTI_ASSIGNMENT_COMMA,
    DECLARATION_START,
    STATEMENT_BLOCK_END,
    RELATIONAL_CLOSE,
    CAST_TYPE_ARGUMENTS_OPEN,
    CLOSURE_PARAMETERS_START,
    TYPE_ONLY_CLOSE,
    BODY_BLOCK_END,
    JUMP_STATEMENT_END,
    CLOSURE_CALL_START,
    CLOSURE_COMMAND_START,
    UNREACHABLE,
    TYPE_ARGUMENTS_OPEN,
    CAST_WORD_CLOSE,
    ERROR_SENTINEL,
};

// SPECIFICATION.md §6.6 — a `/*` with no `*/` after it is not a
// comment, and finding that out means reading to the end of input.
// Without a memory, a file with N of them was read N times.
//
// The fact is about the input alone: no `*/` at or after one position
// means none after any later one. But a scan that fails leaves no
// token, and the runtime restores the state from the last external
// token before every call, so the scan that learns it emits the empty
// UNTERMINATED_COMMENT token to hold it. That token read to the end of
// input, so any edit after it makes the runtime lex it again, and a
// node lexed in this state is reused only where the new parse is in
// the same state.
// §6.6 records the one reparse this does not cover: being empty, the
// token lets a node before it be reused after the comment is closed.
//
// SPECIFICATION.md §6.19 — the scan after a lambda body's `}` that decides
// an expression continues may have read past the start of the next token,
// a comment or `as` or `in`, which a scan that has marked the end at the
// `}` can no longer return. It emits the empty UNTERMINATED_COMMENT, the
// one extra that is no comment, to set `lambda_body_continues`, and the
// next call, at the same position, reads that token without deciding
// again and clears the flag.
//
// SPECIFICATION.md §6.23 — so does the scan after `return`, `break` or
// `continue` that read past a block comment and the spaces after it to
// something other than a line break: the comment's end is passed, and the
// empty token sets `jump_continues`, which the next call clears.
typedef struct {
    bool no_comment_close_ahead;
    bool lambda_body_continues;
    bool jump_continues;
    bool slashy_after_comment_newline;
} Scanner;

// A failed allocation leaves the scanner without its memory, which
// costs time and nothing else.
static inline bool no_comment_close_ahead(const Scanner *scanner) {
    return scanner != NULL && scanner->no_comment_close_ahead;
}

// Wider than the grammar's `identifier` token (SPECIFICATION.md
// §5.2): every non-ASCII code point counts as an identifier
// character, because the scanner has no Unicode tables. That is safe
// because the scanner never emits a name. It only looks ahead: past the
// name in a declaration header, at the key of a named argument, and at
// words it compares with ASCII keywords. The in-grammar lexer then
// reads any name with the real class. On valid input the two agree:
// outside strings and comments, a non-ASCII character can only be part
// of a name. On other input the name the lexer reads is shorter, and
// the parse fails there.
static inline bool is_identifier_start(int32_t c) {
    return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z')
        || c == '_' || c == '$' || c > 0x7F;
}

static inline bool is_identifier_char(int32_t c) {
    return is_identifier_start(c) || (c >= '0' && c <= '9');
}

static inline void advance(TSLexer *lexer) {
    lexer->advance(lexer, false);
}

static inline bool is_whitespace(int32_t c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\r';
}

static inline bool is_inline_space(int32_t c) {
    return c == ' ' || c == '\t';
}

static inline bool is_line_break(int32_t c) {
    return c == '\n' || c == '\r';
}

// SPECIFICATION.md §5.10 — a line escape, `\` followed by a line
// break, is whitespace to Groovy's lexer (GroovyLexer.g4
// `WS : ([ \t]+ | LineEscape+)`), so the line after it continues the
// current one. Skips spaces, tabs and line escapes. Returns false at a
// `\` that does not start a line escape: the `\` is consumed by then
// and no token starts with one, so the caller must return false to
// make the lexer rewind.
static bool skip_same_line_space(TSLexer *lexer, bool skip) {
    for (;;) {
        if (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
            lexer->advance(lexer, skip);
            continue;
        }
        if (lexer->lookahead != '\\') {
            return true;
        }
        lexer->advance(lexer, skip);
        if (lexer->lookahead == '\r') {
            lexer->advance(lexer, skip);
        }
        if (lexer->lookahead != '\n') {
            return false;
        }
        lexer->advance(lexer, skip);
    }
}

// Consumes a name and keeps its first `size` characters in `word`, to
// compare with a keyword. A non-ASCII character is stored as 0, which
// matches no keyword; cut to a `char` it could. Returns the full length.
static unsigned read_word(TSLexer *lexer, char *word, unsigned size) {
    unsigned len = 0;
    // EOF-bounded: lookahead is 0 at end of input, which fails the
    // predicate.
    while (is_identifier_char(lexer->lookahead)) {
        if (len < size) {
            word[len] = lexer->lookahead <= 0x7F ? (char)lexer->lookahead : 0;
        }
        len++;
        lexer->advance(lexer, false);
    }
    return len;
}

// Whether a word that `read_word` returned is `keyword`. A word longer
// than the buffer differs in length from every keyword compared.
static bool word_is(const char *word, unsigned len, const char *keyword) {
    return len == strlen(keyword) && memcmp(word, keyword, len) == 0;
}

// Whether a word that `read_word` returned is the literal `true`,
// `false` or `null`.
static bool is_literal_word(const char *word, unsigned len) {
    return word_is(word, len, "true") || word_is(word, len, "false")
        || word_is(word, len, "null");
}

// SPECIFICATION.md §5.7 — whether the word that `read_word` just returned
// is the `$` of `$/`, which opens a dollar-slashy string. `$` is an
// identifier start, so every look-ahead reads it as a name, but the
// in-grammar lexer takes `$/` wherever a string may start, and what
// follows is the string's body. A `/*` there opens no comment, and no
// scan reaches it to set the §6.6 bit (§6.10), so a look-ahead that read
// on took the body of `$/*.txt/$` for a comment and read to the next `*/`
// or to the end of input, once per string. A longer name that ends in `$`
// is a name (`a$/b` divides), as in Groovy's lexer.
static inline bool opens_dollar_slashy(
    const char *word,
    unsigned len,
    TSLexer *lexer
) {
    return len == 1 && word[0] == '$' && lexer->lookahead == '/';
}

// Consumes a name whose spelling the caller does not need, and returns
// whether it opens a dollar-slashy string instead (`opens_dollar_slashy`).
static bool read_dollar_slashy_open(TSLexer *lexer) {
    char word[sizeof "$"];
    unsigned len = read_word(lexer, word, sizeof word);
    return opens_dollar_slashy(word, len, lexer);
}

// The most characters of a dollar-slashy string that a look-ahead reads
// for its closing `/$`. A string's opener and its closer differ, so no
// later opener closes one that does not close: in a nest of interpolations
// that each open another (`$/ ${ (a = $/ ${ (a = $/ …`) every look-ahead
// would read to the end of input. With the bound each reads this much, and
// a longer string ends the look-ahead as one that does not close does. The
// strings a look-ahead meets are a default value or an annotation's
// argument, a pattern or a path of a few dozen characters.
static const unsigned DOLLAR_SLASHY_LOOKAHEAD_CHARACTERS = 1024;

// Reads a dollar-slashy string from the `/` of its `$/`, which
// `opens_dollar_slashy` has seen, to past its closing `/$`, as
// `_dollar_slashy_string` in grammar.js lexes it: `$$` and `$/` are
// escapes, and of a run of slashes before `$` the last one closes. An
// interpolation in braces is read to the `}` that matches its `{`, so that
// a `/$` inside it closes nothing (`$/ ${f($/a/$)} /$`): braces are
// counted, but for those in a quoted string (`${ '{' }`), read to its
// quote with `\` taking the next character, and nothing else of the
// expression is told apart. `*interpolated`, when not NULL, is set at the
// `$` of an interpolation, which a name or `{` follows. Returns false at
// the end of input and after DOLLAR_SLASHY_LOOKAHEAD_CHARACTERS characters.
static bool skip_dollar_slashy_string(TSLexer *lexer, bool *interpolated) {
    advance(lexer);
    // The braces open in an interpolation, and the quote of a string open
    // in one.
    unsigned braces = 0;
    int32_t quote = 0;
    for (unsigned read = 0; read < DOLLAR_SLASHY_LOOKAHEAD_CHARACTERS; read++) {
        if (lexer->eof(lexer)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (quote != 0) {
            if (c == '\\' && !lexer->eof(lexer)) {
                advance(lexer);
            } else if (c == quote) {
                quote = 0;
            }
        } else if (braces > 0) {
            if (c == '{') {
                braces++;
            } else if (c == '}') {
                braces--;
            } else if (c == '\'' || c == '"') {
                quote = c;
            }
        } else if (c == '$') {
            int32_t next = lexer->lookahead;
            if (next == '$' || next == '/') {
                advance(lexer);
            } else if (next == '{' || is_identifier_start(next)) {
                if (interpolated != NULL) {
                    *interpolated = true;
                }
                if (next == '{') {
                    advance(lexer);
                    braces = 1;
                }
            }
        } else if (c == '/' && lexer->lookahead == '$') {
            advance(lexer);
            return true;
        }
    }
    return false;
}

// Whether the dollar-slashy string that starts here, at the `/` of its
// `$/`, is the name of a method declared with a qualified return type
// (`java.util.List $/foo/$() { }`), as `method_name_follows` tells for a
// quoted one: it closes, has no interpolation, and `(` follows it on the
// line. Groovy reads an interpolated name before `(` as a dynamic call.
static bool dollar_slashy_method_name_follows(TSLexer *lexer) {
    bool interpolated = false;
    return skip_dollar_slashy_string(lexer, &interpolated) && !interpolated
        && skip_same_line_space(lexer, false) && lexer->lookahead == '(';
}

// SPECIFICATION.md §6.15 — the element type of an array type that is a
// value is a primitive type or a class, and class names are capitalized
// by convention, as `scan_command_path_start` also assumes. Checking the
// spelling lets a lowercase name, nearly always a variable or a method,
// be turned down after a character or two instead of being read twice.
static inline bool is_capital(int32_t c) {
    return c >= 'A' && c <= 'Z';
}

// Whether a primitive type name or a class name may start with `c`.
static inline bool is_element_type_start(int32_t c) {
    return is_capital(c) || c == 'b' || c == 'c' || c == 'd' || c == 'f'
        || c == 'i' || c == 'l' || c == 's';
}

// Whether a word that `read_word` returned is a primitive type name.
static bool is_primitive_type(const char *word, unsigned len) {
    return word_is(word, len, "boolean") || word_is(word, len, "byte")
        || word_is(word, len, "char") || word_is(word, len, "double")
        || word_is(word, len, "float") || word_is(word, len, "int")
        || word_is(word, len, "long") || word_is(word, len, "short");
}

// Consumes a name and returns whether it is a primitive type name. It
// stops at the first character that no such name has there, so most
// names cost a character or two.
static bool read_primitive_type(TSLexer *lexer) {
    const char *first = NULL;
    const char *second = NULL;
    switch (lexer->lookahead) {
        case 'b':
            first = "boolean";
            second = "byte";
            break;
        case 'c':
            first = "char";
            break;
        case 'd':
            first = "double";
            break;
        case 'f':
            first = "float";
            break;
        case 'i':
            first = "int";
            break;
        case 'l':
            first = "long";
            break;
        case 's':
            first = "short";
            break;
        default:
            return false;
    }
    unsigned i = 0;
    // EOF-bounded: lookahead is 0 at end of input, which fails the
    // predicate. A candidate is dropped at its terminating NUL, which no
    // identifier character equals.
    for (; is_identifier_char(lexer->lookahead); i++) {
        if (first != NULL && first[i] != lexer->lookahead) {
            first = NULL;
        }
        if (second != NULL && second[i] != lexer->lookahead) {
            second = NULL;
        }
        if (first == NULL && second == NULL) {
            return false;
        }
        lexer->advance(lexer, false);
    }
    return (first != NULL && first[i] == '\0')
        || (second != NULL && second[i] == '\0');
}

// Consumes `.Name` for as long as one follows and its name is
// capitalized, as the names of nested types are (`Map.Entry`). Returns
// false, with input consumed, at a `.` that no capitalized name follows.
static bool read_nested_type_names(TSLexer *lexer) {
    while (lexer->lookahead == '.') {
        lexer->advance(lexer, false);
        if (!is_capital(lexer->lookahead)) {
            return false;
        }
        read_word(lexer, NULL, 0);
    }
    return true;
}

// Consumes a name and returns whether it may name the element type of an
// array type: a primitive type name, or a capitalized name and any nested
// type names after it. A lowercase name is read only while it can still
// become a primitive type name.
static bool read_element_type_name(TSLexer *lexer) {
    if (is_capital(lexer->lookahead)) {
        read_word(lexer, NULL, 0);
        return read_nested_type_names(lexer);
    }
    return read_primitive_type(lexer);
}

// Defined with the other look-ahead helpers below.
static bool skip_header_trivia(
    TSLexer *lexer,
    bool newlines,
    bool no_close_ahead,
    bool *crossed_line,
    bool *slash
);
static bool array_dimensions_follow(
    TSLexer *lexer,
    bool type_may_start,
    bool no_close_ahead
);
static bool is_contextual_word(const char *word, unsigned len);
static inline bool contextual_word_may_start(
    int32_t c,
    const bool *valid_symbols
);
static bool slashy_body_follows(TSLexer *lexer, bool lines);
static bool skip_string(TSLexer *lexer);
static bool skip_short_string(TSLexer *lexer);
static bool lambda_parameters_follow(
    TSLexer *lexer,
    bool no_close_ahead,
    bool *closed,
    bool *in_string
);
static bool scan_contextual_word(
    TSLexer *lexer,
    const char *word,
    unsigned len,
    const bool *valid_symbols,
    bool no_close_ahead
);

// Whether the name that `read_word` just consumed is the key of a named
// argument: `:` follows on the same line, and not `::`, which makes a
// method reference. `default:` is a switch label, not a key. Apache's
// `namedArgLabel` accepts the keyword, so `foo default: 1` diverges on
// purpose: the one-liner `switch (x) { case 1: foo default: bar }` in
// statements-switch.txt depends on the rejection. `case` is a key
// (`foo case: 1`), as in Groovy: a `case` label needs a value before its
// `:`, and the grammar reserves the word, so no `case:` is a label.
static bool key_colon_follows(TSLexer *lexer, const char *word, unsigned len) {
    if (word_is(word, len, "default")) {
        return false;
    }
    if (!skip_same_line_space(lexer, false)) {
        return false;
    }
    if (lexer->lookahead != ':') {
        return false;
    }
    lexer->advance(lexer, false);
    return lexer->lookahead != ':';
}

// Whether a member access may start here: the first character of every
// operator `read_member_access` reads.
static inline bool is_member_access_start(int32_t c) {
    return c == '.' || c == '?' || c == '*';
}

// Consumes one of the operators that join the names of a
// parenthesis-free call's receiver: `.`, `?.`, `??.`, `*.`, `.&`, `.@`,
// `?.@`, `??.@` and `*.@`, and clears `dot` unless it is `.`. Returns
// false, with input consumed, on anything else.
static bool read_member_access(TSLexer *lexer, bool *dot) {
    if (lexer->lookahead != '.') {
        *dot = false;
    }
    switch (lexer->lookahead) {
        case '.':
            lexer->advance(lexer, false);
            if (lexer->lookahead == '&' || lexer->lookahead == '@') {
                *dot = false;
                lexer->advance(lexer, false);
            }
            return true;
        case '?':
            lexer->advance(lexer, false);
            if (lexer->lookahead == '?') {
                lexer->advance(lexer, false);
            }
            break;
        case '*':
            lexer->advance(lexer, false);
            break;
        default:
            return false;
    }
    if (lexer->lookahead != '.') {
        return false;
    }
    lexer->advance(lexer, false);
    if (lexer->lookahead == '@') {
        lexer->advance(lexer, false);
    }
    return true;
}

// Consumes member accesses, each an operator and a name, for as long as
// one starts here, and sets `last_name` to the first character of the last
// name read. Sets `nested` to whether every access is `.` and every name
// after one is capitalized, as in the name of a nested type, and
// `ends_in_new` to whether the last name is `new`. Returns false, with
// input consumed, at an operator that is not one of them or that no name
// follows.
static bool read_member_accesses(
    TSLexer *lexer,
    int32_t *last_name,
    bool *nested,
    bool *ends_in_new
) {
    *nested = true;
    *ends_in_new = false;
    while (is_member_access_start(lexer->lookahead)) {
        if (!read_member_access(lexer, nested)
            || !is_identifier_start(lexer->lookahead)) {
            return false;
        }
        *last_name = lexer->lookahead;
        if (!is_capital(*last_name)) {
            *nested = false;
        }
        char name[sizeof "new"];
        *ends_in_new = word_is(name, read_word(lexer, name, sizeof name), "new");
    }
    return true;
}

// SPECIFICATION.md §6.8 — decides that the key of a named argument
// in a parenthesis-free call (`exclude group: 'a'`) starts here: a
// name on the current line, followed by `:` and not `::`. The token is
// empty. `mark_end` fixes it at the start of the name, and the
// in-grammar lexer reads the name itself, so the key is an identifier
// by the grammar's own definition, non-ASCII letters included. A
// newline before the key means a new statement (a Spock `where:`
// label, say), so the caller does not skip one.
//
// An argument after a comma may also be an array type (§6.15,
// `foo 'a', String[]`), which the same name starts. The scanner cannot
// read the name twice, so ARRAY_VALUE_START is decided here as well when
// `wants_array` is set.
//
// §6.16 — the same name may be `var`, `yield`, `in` or `as`, after a
// statement's first name (`it in xs`, `it as T`). Its keyword must end
// at the word, and whether it is a key is known only past the spaces
// after it (`foo in : 1`), so the word ends the token in either case:
// such a key is CONTEXTUAL_KEY, which spans the word and which the
// grammar aliases to `identifier`.
//
// The caller has skipped the same-line space and seen an identifier
// start.
static bool scan_command_named_key(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool wants_array,
    bool type_may_start,
    bool no_close_ahead
) {
    lexer->mark_end(lexer);
    bool capitalized = is_capital(lexer->lookahead);
    char word[sizeof "default"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (is_contextual_word(word, len)) {
        lexer->mark_end(lexer);
        if (!skip_same_line_space(lexer, false)) {
            return false;
        }
        if (lexer->lookahead == ':') {
            lexer->advance(lexer, false);
            lexer->result_symbol = CONTEXTUAL_KEY;
            return valid_symbols[CONTEXTUAL_KEY] && lexer->lookahead != ':';
        }
        return scan_contextual_word(lexer, word, len, valid_symbols,
                                    no_close_ahead);
    }
    if (lexer->lookahead == '.') {
        // A key is one name; only a nested type name can go on.
        if (!wants_array || !capitalized || !read_nested_type_names(lexer)
            || !array_dimensions_follow(lexer, type_may_start, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = ARRAY_VALUE_START;
        return true;
    }
    if (key_colon_follows(lexer, word, len)) {
        lexer->result_symbol = COMMAND_NAMED_KEY;
        return true;
    }
    // Groovy's lexer hides a block comment that something follows on its
    // line, so the name is a key past one (`foo a /* c */ : 1` is
    // `foo(a: 1)`), even one that spans lines. `key_colon_follows` stops at
    // the comment, and the look-aheads that read a name argument
    // (`name_argument_rest`) take the `:` after it for the end of the
    // argument, so the call is open here. At a `/` no array type follows.
    // The `/` after a lone `$` opens a dollar-slashy string, whose body is
    // no comment (`println $/*.txt/$`).
    if (lexer->lookahead == '/' && !opens_dollar_slashy(word, len, lexer)) {
        if (!skip_header_trivia(lexer, false, no_close_ahead, NULL, NULL)
            || !key_colon_follows(lexer, word, len)) {
            return false;
        }
        lexer->result_symbol = COMMAND_NAMED_KEY;
        return true;
    }
    if (wants_array && lexer->lookahead == '['
        && (capitalized || is_primitive_type(word, len))
        && array_dimensions_follow(lexer, type_may_start, no_close_ahead)) {
        lexer->result_symbol = ARRAY_VALUE_START;
        return true;
    }
    return false;
}

// Whether the `new` just consumed opens an object or array creation: the
// name of the created type follows, after Groovy's trivia, which
// `new nls creator` in GroovyParser.g4 allows to include line breaks.
// `x.y new` followed by `(c)` on the next line is not one.
static bool creation_follows(TSLexer *lexer, bool no_close_ahead) {
    return skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
        && is_identifier_start(lexer->lookahead);
}

// SPECIFICATION.md §6.15 — whether the name that `read_word` just
// consumed starts an array type that is a value (`String[].class`,
// `int[]::new`, `Map.Entry[].class`): a capitalized name and any nested
// type names, or a primitive type name, then dimensions followed by a
// member access or `::`. A bare `String[]` is a type, as where a type
// may start.
static bool array_value_follows(
    TSLexer *lexer,
    const char *word,
    unsigned len,
    bool capitalized,
    bool no_close_ahead
) {
    if (lexer->lookahead == '.') {
        if (!capitalized || !read_nested_type_names(lexer)) {
            return false;
        }
    } else if (!capitalized && !is_primitive_type(word, len)) {
        return false;
    }
    return array_dimensions_follow(lexer, true, no_close_ahead);
}

static bool parenthesized_command_follows(
    TSLexer *lexer,
    bool no_close_ahead
);
static bool call_arguments_follow(
    TSLexer *lexer,
    bool commands,
    bool no_close_ahead
);

static bool keyword_starts_operand(
    const char *word,
    unsigned len,
    int32_t next
);
// Defined after is_grammar_keyword, which they call.
static bool is_value_keyword(const char *word, unsigned len, int32_t next);
static bool is_reserved_lambda_name(const char *word, unsigned len);
static bool is_reserved_bare_lambda_name(const char *word, unsigned len);

// SPECIFICATION.md §6.14 — the most the look-ahead after a name argument
// reads of a list (`parameters_follow`, `call_arguments_follow`), in items:
// a word, a number, a string, a comment, a line break or a punctuation
// character. Malformed input can restart a statement at every element of a
// long list (`{ bar a, bar b, … }` with no `->`), and each restart reads
// ahead from there, so without a bound the parse takes time quadratic in
// the list. With it each restart reads at most this many items, and the
// parse stays linear. A list longer than this after a lowercase-typed
// first closure parameter is read as a call's arguments; real lists are a
// few items.
static const unsigned NAME_ARGUMENT_LOOKAHEAD_ITEMS = 256;

static bool is_modifier_word(const char *word, unsigned len);
static bool is_reserved_variable_name(const char *word, unsigned len);
static bool type_parameters_follow(TSLexer *lexer);

// The code points of the upper-case letters `is_upper_letter` knows
// beyond A to Z: the capitals of Latin-1 (`À` to `Þ`, but for the
// multiplication sign `×`), of Greek (`Α` to `Ω`, but for the unassigned
// code point between `Ρ` and `Σ`) and of Cyrillic (`Ѐ` to `Я`).
enum {
    LATIN1_CAPITAL_FIRST = 0xC0,
    LATIN1_CAPITAL_LAST = 0xDE,
    MULTIPLICATION_SIGN = 0xD7,
    GREEK_CAPITAL_FIRST = 0x391,
    GREEK_CAPITAL_LAST = 0x3A9,
    GREEK_CAPITAL_UNASSIGNED = 0x3A2,
    CYRILLIC_CAPITAL_FIRST = 0x400,
    CYRILLIC_CAPITAL_LAST = 0x42F,
};

// SPECIFICATION.md §6.14 — whether `c` is an upper-case letter, which
// Apache's isInvalidLocalVariableDeclaration asks of a receiver's first
// letter (`Character.isUpperCase`): A to Z, and the capitals of Latin-1,
// Greek and Cyrillic. A capital of any other script reads as lower case;
// the scanner has no Unicode tables.
static inline bool is_upper_letter(int32_t c) {
    return is_capital(c)
        || (c >= LATIN1_CAPITAL_FIRST && c <= LATIN1_CAPITAL_LAST
            && c != MULTIPLICATION_SIGN)
        || (c >= GREEK_CAPITAL_FIRST && c <= GREEK_CAPITAL_LAST
            && c != GREEK_CAPITAL_UNASSIGNED)
        || (c >= CYRILLIC_CAPITAL_FIRST && c <= CYRILLIC_CAPITAL_LAST);
}

// SPECIFICATION.md §6.14 — whether a receiver, one name that `read_word`
// returned, may instead be the type of a declaration: a primitive type name,
// `void` or a name with an upper-case first letter (`upper`). That is how
// Apache's isInvalidLocalVariableDeclaration (the same from Groovy 3 to 6)
// tells `String s` and `int x` from the call `foo bar`; for a dotted name it
// looks at the last name (`foo.Bar x`, `path_takes_names`). Such a receiver
// takes no name as its first argument, in a value as well, so
// `Point(int x)` and `foo(String a)` hold no call.
static bool may_name_type(const char *word, unsigned len, bool upper) {
    return upper || is_primitive_type(word, len) || word_is(word, len, "void");
}

// Whether a receiver that ends in member accesses, read by
// `read_member_accesses`, takes a name as its first argument: its last name
// does not start with an upper-case letter (`a.b c`, but `foo.Bar x`
// declares) and is not `new` (`this.new Inner()` creates an inner class's
// instance, Apache `DOT NEW creator`).
static inline bool path_takes_names(int32_t last_name, bool ends_in_new) {
    return !is_upper_letter(last_name) && !ends_in_new;
}

// After `!`, which the caller has seen: a negation, the first argument of a
// call (`foo !x`), unless the operator is `!=`, or `!in` or `!instanceof`
// before what makes them operators (§6.12): whitespace or, after `!in`, a
// bracket.
static bool negation_follows(TSLexer *lexer) {
    advance(lexer);
    if (lexer->lookahead == '=') {
        return false;
    }
    if (lexer->lookahead != 'i') {
        return true;
    }
    char word[sizeof "instanceof"];
    unsigned len = read_word(lexer, word, sizeof word);
    int32_t next = lexer->lookahead;
    if (word_is(word, len, "in")) {
        return !is_whitespace(next) && next != '[' && next != '('
            && next != '{';
    }
    return !(word_is(word, len, "instanceof") && is_whitespace(next));
}

// Whether a name that `read_word` returned, with `next` the character after
// it, may be a call's first argument: no keyword Groovy reserves, and
// neither `in` nor `as`, which continue the receiver as operators.
static bool name_may_be_argument(const char *word, unsigned len, int32_t next) {
    return !is_value_keyword(word, len, next) && !word_is(word, len, "in")
        && !word_is(word, len, "as");
}

// Whether a name argument spelled `word` starts what Groovy may read as a
// declaration or a statement of its own when a name follows it: `var`,
// `yield`, `record`, `trait` and `sealed`. `foo var x` is left as before,
// `foo` and the declaration `var x`; Groovy reads `foo(var).x`.
static bool is_declaring_word(const char *word, unsigned len) {
    return word_is(word, len, "var") || word_is(word, len, "yield")
        || word_is(word, len, "record") || word_is(word, len, "trait")
        || word_is(word, len, "sealed");
}

// Whether a number or a string starts with `c`.
static inline bool is_literal_start(int32_t c) {
    return (c >= '0' && c <= '9') || c == '\'' || c == '"';
}

static bool first_argument_follows(
    TSLexer *lexer,
    bool parenthesized,
    bool names,
    bool statement,
    bool no_close_ahead
);

static bool first_argument_word_follows(
    TSLexer *lexer,
    const char *argument,
    unsigned argument_len,
    bool capitalized,
    bool parenthesized,
    bool names,
    bool statement,
    bool no_close_ahead
);

// The trivia after a name argument, which Groovy's lexer hides: spaces,
// tabs, line escapes and block comments, which may span lines; a line
// comment is read to the end of its line. Returns false where the look-ahead
// must stop: at a `/` that opens no comment (division, consumed), and at a
// block comment that does not close (§6.6). A comment is no trivia between
// the receiver and the name argument (§6.14), only after the argument.
static bool skip_argument_trivia(TSLexer *lexer, bool no_close_ahead) {
    bool slash = false;
    if (!skip_header_trivia(lexer, false, no_close_ahead, NULL, &slash)) {
        return false;
    }
    if (!slash) {
        return true;
    }
    if (lexer->lookahead != '/') {
        return false;
    }
    while (!lexer->eof(lexer) && !is_line_break(lexer->lookahead)) {
        advance(lexer);
    }
    return true;
}

// After a name argument that `.` or `[` follows (`foo x.y`, `foo xs[0]`):
// its member accesses, then on the line no first argument as the grammar
// takes it after a name (see `continuation_follows`). In Groovy
// `foo x.y 'a'` is `foo(x.y).a`, a chain continuation, which is left as
// before: `foo` and the call `x.y 'a'`. A subscript is not read past, so
// `foo xs[0] 1` is the call `foo xs[0]` and the statement `1`.
static bool dotted_argument_rest(TSLexer *lexer, bool no_close_ahead) {
    int32_t last_name = 0;
    bool nested = false;
    bool ends_in_new = false;
    if (!read_member_accesses(lexer, &last_name, &nested, &ends_in_new)) {
        return true;
    }
    return !first_argument_follows(lexer, false, false, false, no_close_ahead);
}

// After a name argument, at a name that follows it: whether that name
// starts a chain continuation, which is left as before (§5.5 item 4) —
// a literal word, a dollar-slashy string, a key, a creation or an array
// type value, the first argument as the grammar takes it after a name
// (`foo bar true`, `foo bar $/a/$`, `foo bar baz: 1`, `foo bar new Baz()`,
// `foo bar String[].class`), or a
// name and `=` (`foo bar x = 1`, Groovy's `foo.bar(x = 1)`); in a value
// any name (`def r = a b c` is Groovy's `a(b).c`, and the rest would be no
// member of a class body). After `var`, `yield`, `record`, `trait` or
// `sealed` (`declaring`) any name does. So does a name before parentheses
// that hold parameters or a parenthesis-free call (`call_arguments_follow`):
// `foo bar baz(int a) { }` stays `foo` and a method, which Groovy reads as
// `foo(bar).baz(…)`; inside the one parenthesis already read (`nested`)
// that is not looked into. `in`, `as` and `instanceof` go on with the
// argument as operators, unless a key (`foo bar as: 1`).
static bool continuation_follows(
    TSLexer *lexer,
    bool statement,
    bool declaring,
    bool nested,
    bool no_close_ahead
) {
    bool capitalized = is_capital(lexer->lookahead);
    char word[sizeof "instanceof"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (word_is(word, len, "in") || word_is(word, len, "as")
        || word_is(word, len, "instanceof")) {
        return key_colon_follows(lexer, word, len);
    }
    if (declaring || is_literal_word(word, len)
        || opens_dollar_slashy(word, len, lexer)) {
        return true;
    }
    if (word_is(word, len, "new")) {
        return key_colon_follows(lexer, word, len)
            || creation_follows(lexer, no_close_ahead);
    }
    if (lexer->lookahead == '.' || lexer->lookahead == '[') {
        return array_value_follows(lexer, word, len, capitalized,
                                   no_close_ahead)
            || !statement;
    }
    if (key_colon_follows(lexer, word, len)) {
        return true;
    }
    if (lexer->lookahead == '=') {
        advance(lexer);
        return lexer->lookahead != '=' && lexer->lookahead != '~';
    }
    if (lexer->lookahead == '(' && !nested) {
        return !call_arguments_follow(lexer, true, no_close_ahead);
    }
    return !statement;
}

// At the end of closure parameters, past a line break or a comment: whether
// `->` follows after Groovy's trivia (whitespace, line breaks and comments),
// as Groovy allows them before it. The trivia is read once per call; no
// statement can start inside it, so no restart reads it again.
static bool arrow_follows(TSLexer *lexer, bool no_close_ahead) {
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
        || lexer->lookahead != '-') {
        return false;
    }
    advance(lexer);
    return lexer->lookahead == '>';
}

// Reads a block comment from its `*`, the `/` before it consumed, to its
// close. Returns false at the end of input, where it does not close.
static bool skip_comment_body(TSLexer *lexer) {
    advance(lexer);
    for (;;) {
        if (lexer->eof(lexer)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '*' && lexer->lookahead == '/') {
            advance(lexer);
            return true;
        }
    }
}

static bool next_line_continues(
    TSLexer *lexer,
    bool slash,
    bool declaration,
    bool no_close_ahead
);

// How the line after a list element that may be complete starts, past
// Groovy's trivia (`default_line_follows`).
enum {
    LINE_ENDS,
    LINE_GOES_ON,
    LINE_ARROW,
};

// At a line break after a list element that may be complete (`foo a, b`
// then a line): LINE_ARROW when `->` follows, which ends closure
// parameters; LINE_GOES_ON when the line goes on with the element, as
// Groovy lets a line open with most binary operators, `?`, `:` and a member
// access (`next_line_continues`, and `=` with the operators that start with
// it, `in` and `as`: `{ foo a, b = i++` then `* 2 -> a }`); and LINE_ENDS
// when it starts the next statement. A `(` or `,` that opens the line ends
// the element, as it ends a call's last argument.
//
// After `var` that starts an element (`after_var`), the name it declares
// may open the line, as Apache's `variableModifiers` takes `nls` after each
// modifier (`{ String a = 1, var` then `b -> a }`): LINE_ARROW where `->`
// follows that name on its line and LINE_GOES_ON where `,` or `=` does.
// Any other line ends the element, as `var` is a name too, a call's last
// argument (`foo a, var` then the next statement).
static int default_line_follows(
    TSLexer *lexer,
    bool after_var,
    bool no_close_ahead
) {
    bool slash = false;
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, &slash)) {
        return LINE_ENDS;
    }
    int32_t c = lexer->lookahead;
    if (!slash) {
        if (c == '-') {
            advance(lexer);
            if (lexer->lookahead == '>') {
                return LINE_ARROW;
            }
            return lexer->lookahead == '=' ? LINE_GOES_ON : LINE_ENDS;
        }
        if (c == '=') {
            return LINE_GOES_ON;
        }
        if (c == '(' || c == ',') {
            return LINE_ENDS;
        }
        if (is_identifier_start(c)) {
            char word[sizeof "synchronized"];
            unsigned len = read_word(lexer, word, sizeof word);
            if (word_is(word, len, "in") || word_is(word, len, "as")
                || word_is(word, len, "instanceof")) {
                return LINE_GOES_ON;
            }
            if (!after_var || is_reserved_variable_name(word, len)
                || !skip_same_line_space(lexer, false)) {
                return LINE_ENDS;
            }
            if (lexer->lookahead == ',' || lexer->lookahead == '=') {
                return LINE_GOES_ON;
            }
            if (lexer->lookahead != '-') {
                return LINE_ENDS;
            }
            advance(lexer);
            return lexer->lookahead == '>' ? LINE_ARROW : LINE_ENDS;
        }
    }
    return next_line_continues(lexer, slash, false, no_close_ahead)
        ? LINE_GOES_ON
        : LINE_ENDS;
}

// After the `,` of `foo a,` in a statement: whether `->` ends the list,
// which makes it a closure's parameters, the first of type `foo`
// (`{ foo a, b -> }`, `{ foo a, @A b = 1 -> }`). The elements are read as
// Groovy's tokens, bracket depth counted, so that a default value may hold
// anything; comments are trivia, and a string is one item, a triple-quoted
// one across lines (`skip_string`) and a dollar-slashy one as well
// (`skip_dollar_slashy_string`). A line break ends the list unless a comma,
// a modifier, an annotation, an operator or an open bracket comes before it
// (a postfix `++` or `--` and a slashy string end an operand), or `->` or a
// line that goes on with the element after it (`default_line_follows`): a
// call's last argument ends at it (`foo a, i++` then the next statement).
// An annotation is `@`, its dotted name and arguments in parentheses, with
// no bracket open: the parameter it stands on may follow on the next line
// (`{ String a,` then `@A` then `String b -> }`). `;`, a closing bracket
// with none open and the end of the
// budget, `budget` items, end the look-ahead with no `->`. So does an
// element, the first item after a `,` outside brackets, that is `(` or a
// key (`name:`), which starts no closure parameter: the `->` after it is a
// lambda's, and the list a call's arguments (`foo a, () -> b`,
// `foo a, k: (x) -> b`). §6.21 reads the same after the `=` of a declared
// name (`{ String a = 1 -> }`).
static bool parameters_follow(
    TSLexer *lexer,
    unsigned budget,
    bool no_close_ahead
) {
    unsigned depth = 0;
    bool continued = true;
    // Whether the item read next starts an element: the last item read, line
    // breaks and comments aside, is a `,` outside brackets.
    bool element_start = false;
    // An annotation outside brackets: whether the last item is the `@` or
    // a `.` of its name, whether it is a part of that name, and whether its
    // arguments are open.
    bool annotation_prefix = false;
    bool annotation_name = false;
    bool annotation_arguments = false;
    // Whether the last item on this line is `var` that starts an element.
    bool var_word = false;
    for (unsigned items = 0; items < budget; items++) {
        if (!skip_same_line_space(lexer, false) || lexer->eof(lexer)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        bool after_var = var_word;
        var_word = false;
        bool after_annotation_prefix = annotation_prefix;
        annotation_prefix = false;
        bool after_annotation_name = annotation_name;
        annotation_name = false;
        if (is_line_break(c)) {
            if (depth == 0 && !continued) {
                int line =
                    default_line_follows(lexer, after_var, no_close_ahead);
                if (line != LINE_GOES_ON) {
                    return line == LINE_ARROW;
                }
                continued = true;
                continue;
            }
            advance(lexer);
            continue;
        }
        if (c == '/') {
            advance(lexer);
            if (lexer->lookahead == '/') {
                while (!lexer->eof(lexer) && !is_line_break(lexer->lookahead)) {
                    advance(lexer);
                }
                continue;
            }
            if (lexer->lookahead == '*') {
                if (no_close_ahead || !skip_comment_body(lexer)) {
                    return false;
                }
                continue;
            }
            // Where an operand may start, Groovy's lexer reads a slashy
            // string, one item, which the scan function opens where no `=`
            // follows the `/` and the string closes on its line
            // (`slashy_body_follows`): an `->` in it ends no list
            // (`foo a, /->/`, `{ foo a, b = /->/ }`), and a line break after
            // it ends the list as after any string (`foo a, /re/`).
            if (continued && lexer->lookahead != '='
                && slashy_body_follows(lexer, true)) {
                advance(lexer);
                continued = false;
                element_start = false;
                continue;
            }
            continued = true;
            element_start = false;
            continue;
        }
        // `++` and `--` are one token each, as Groovy's lexer reads them, so
        // the `--` of `i-->0` opens no `->`. A postfix update ends its
        // operand, and a line break after it ends the list (`foo a, i++`
        // then the next statement); Groovy rejects one after a prefix update.
        if (c == '+' || c == '-') {
            advance(lexer);
            if (lexer->lookahead == c) {
                advance(lexer);
                continued = false;
                element_start = false;
                continue;
            }
            if (c == '-' && lexer->lookahead == '>' && depth == 0) {
                return true;
            }
            continued = true;
            element_start = false;
            continue;
        }
        if (c == '\'' || c == '"') {
            if (!skip_string(lexer)) {
                return false;
            }
            continued = false;
            element_start = false;
            continue;
        }
        if (is_identifier_char(c)) {
            char word[sizeof "synchronized"];
            unsigned len = read_word(lexer, word, sizeof word);
            if (opens_dollar_slashy(word, len, lexer)) {
                if (!skip_dollar_slashy_string(lexer, NULL)) {
                    return false;
                }
                continued = false;
                element_start = false;
                continue;
            }
            if (element_start && key_colon_follows(lexer, word, len)) {
                return false;
            }
            annotation_name = after_annotation_prefix;
            continued = annotation_name || is_modifier_word(word, len)
                || word_is(word, len, "def");
            var_word = element_start && word_is(word, len, "var");
            element_start = false;
            continue;
        }
        if (c == '(' && element_start) {
            return false;
        }
        if (c == '(' || c == '[' || c == '{') {
            if (c == '(' && depth == 0 && after_annotation_name) {
                annotation_arguments = true;
            }
            depth++;
        } else if (c == ')' || c == ']' || c == '}') {
            if (depth == 0) {
                return false;
            }
            depth--;
        } else if (c == ';' && depth == 0) {
            return false;
        }
        advance(lexer);
        continued = c != ')' && c != ']' && c != '}';
        if (depth == 0) {
            if (c == ')' && annotation_arguments) {
                annotation_arguments = false;
                continued = true;
            }
            annotation_prefix =
                c == '@' || (c == '.' && after_annotation_name);
        }
        element_start = c == ',' && depth == 0;
    }
    return false;
}

// After a name argument and the spaces after it, which `key_colon_follows`
// has skipped: whether the name is all of the first argument's start, which
// it is unless what follows, past comments (`skip_argument_trivia`), makes
// the input something else:
//   - `=` in a statement: `foo bar = 1` declares `bar`, by Apache's rule
//     (`==` and `=~` are operators of the argument);
//   - `->`, or a list that ends in `->`, in a statement, also on a later
//     line: closure parameters, the first of type `foo`;
//   - a literal, or a name that `continuation_follows` takes: in Groovy a
//     further call in the chain, left as before (§5.5 item 4);
//   - `(` with a parenthesis-free call or typed parameters in it: in
//     Groovy `foo bar(baz 1)` is `foo.bar(baz(1))`, and Groovy rejects
//     `foo bar(int a) { }`, which stays a method here. Inside the one
//     parenthesis the look-ahead reads (`nested`), the call is taken as it
//     is, so that no part of the input is read more than a bounded number
//     of times.
// `declaring` is set when the name argument is `var`, `yield`, `record`,
// `trait` or `sealed` (`is_declaring_word`).
static bool name_argument_rest(
    TSLexer *lexer,
    bool statement,
    bool nested,
    bool declaring,
    bool no_close_ahead
) {
    if (!skip_argument_trivia(lexer, no_close_ahead)) {
        return true;
    }
    int32_t c = lexer->lookahead;
    if (c == '(') {
        return nested || call_arguments_follow(lexer, true, no_close_ahead);
    }
    if (is_literal_start(c)) {
        return false;
    }
    if (is_identifier_start(c)) {
        return !continuation_follows(lexer, statement, declaring, nested,
                                     no_close_ahead);
    }
    if (c == '-') {
        advance(lexer);
        return lexer->lookahead != '>';
    }
    if (!statement) {
        return true;
    }
    if (c == ',') {
        return !parameters_follow(lexer, NAME_ARGUMENT_LOOKAHEAD_ITEMS,
                                  no_close_ahead);
    }
    if (c == '=') {
        advance(lexer);
        return lexer->lookahead == '=' || lexer->lookahead == '~';
    }
    if (is_line_break(c)) {
        return !arrow_follows(lexer, no_close_ahead);
    }
    return true;
}

// Whether the first argument of a parenthesis-free call starts here, on
// the line: after spaces, tabs and line escapes, a number, a string,
// `true`, `false`, `null`, an object or array creation, the key of a
// named argument or an array type that is a value; or, when
// `parenthesized` is set, such a call in parentheses
// (`parenthesized_command_follows`). When `names` is set, also a name, as
// `name_may_be_argument` and `name_argument_rest` take it, a negation or
// `~`: Apache's argumentList after any expression that a name, `!` or `~`
// cannot continue. `statement` adds the rules of a statement. A quote is
// left unread. A slashy string is not an argument here: after a name
// Groovy reads `/` as division. A dollar-slashy string is one after any
// receiver, as a quoted string is (`def x = Foo $/a/$`): Groovy's lexer
// asks nothing of what comes before its `$/`.
static bool first_argument_follows(
    TSLexer *lexer,
    bool parenthesized,
    bool names,
    bool statement,
    bool no_close_ahead
) {
    if (!skip_same_line_space(lexer, false)) {
        return false;
    }
    int32_t c = lexer->lookahead;
    if (is_literal_start(c)) {
        return true;
    }
    if (c == '(') {
        return parenthesized
            && parenthesized_command_follows(lexer, no_close_ahead);
    }
    if (names && c == '!') {
        return negation_follows(lexer);
    }
    if (names && c == '~') {
        return true;
    }
    if (!is_identifier_start(c)) {
        return false;
    }
    bool capitalized = is_capital(c);
    char argument[sizeof "synchronized"];
    unsigned argument_len = read_word(lexer, argument, sizeof argument);
    return first_argument_word_follows(lexer, argument, argument_len,
                                       capitalized, parenthesized, names,
                                       statement, no_close_ahead);
}

// `first_argument_follows` past a name, `argument`, which the caller has
// read; `capitalized` says whether it starts with an upper-case letter.
static bool first_argument_word_follows(
    TSLexer *lexer,
    const char *argument,
    unsigned argument_len,
    bool capitalized,
    bool parenthesized,
    bool names,
    bool statement,
    bool no_close_ahead
) {
    if (is_literal_word(argument, argument_len)) {
        return true;
    }
    if (opens_dollar_slashy(argument, argument_len, lexer)) {
        // In a statement whose receiver may name a type, a string before
        // `(` names a method instead, as `scan_command_path_start` tells
        // for a quoted one.
        return !statement || names
            || !dollar_slashy_method_name_follows(lexer);
    }
    if (word_is(argument, argument_len, "new")) {
        return key_colon_follows(lexer, argument, argument_len)
            || creation_follows(lexer, no_close_ahead);
    }
    int32_t next = lexer->lookahead;
    if (names && (next == '.' || next == '[')) {
        return name_may_be_argument(argument, argument_len, next)
            && dotted_argument_rest(lexer, no_close_ahead);
    }
    // A key is one name; only a nested type name can go on.
    if (next != '.') {
        if (key_colon_follows(lexer, argument, argument_len)) {
            return true;
        }
        if (names) {
            return name_may_be_argument(argument, argument_len, next)
                && name_argument_rest(lexer, statement, !parenthesized,
                                      is_declaring_word(argument, argument_len),
                                      no_close_ahead);
        }
        if (lexer->lookahead != '[') {
            return false;
        }
    }
    return array_value_follows(lexer, argument, argument_len, capitalized,
                               no_close_ahead);
}

// SPECIFICATION.md §6.14 — whether a parenthesis-free call in
// parentheses starts here, as the first argument of another one:
// Groovy reads `println(foo 1)` as `println` called with `(foo 1)`
// (Apache `expressionInPar` as the first element of `argumentList`).
// After the `(` and Groovy's trivia, which inside parentheses includes
// line breaks, the receiver and the first argument that
// COMMAND_VALUE_START reads there. That token opens the inner call, and
// the scanner decides it again once the parser is past the `(`. The
// look-ahead goes into one parenthesis only, so that each part of the
// input is read a bounded number of times: `println(bar(foo 1))` is no
// call of `println`.
static bool parenthesized_command_follows(
    TSLexer *lexer,
    bool no_close_ahead
) {
    lexer->advance(lexer, false);
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
        || !is_identifier_start(lexer->lookahead)) {
        return false;
    }
    bool upper = is_upper_letter(lexer->lookahead);
    char word[sizeof "synchronized"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (is_value_keyword(word, len, lexer->lookahead)) {
        return false;
    }
    bool accesses = is_member_access_start(lexer->lookahead);
    int32_t last_name = 0;
    bool nested = false;
    bool ends_in_new = false;
    if (!read_member_accesses(lexer, &last_name, &nested, &ends_in_new)) {
        return false;
    }
    bool names = accesses ? path_takes_names(last_name, ends_in_new)
        : !may_name_type(word, len, upper);
    return first_argument_follows(lexer, false, names, false, no_close_ahead);
}

// The dots of a varargs parameter's `...`, Apache's ELLIPSIS token.
static const unsigned ELLIPSIS_DOTS = 3;

// At the name after a parameter's type, which `parameter_name_follows` has
// reached: whether it names the parameter. `instanceof` does not, nor does
// a word that names no variable (`is_reserved_variable_name`: `(Foo new
// Bar())` holds a call of `Foo`), and nor do `as` and `in` (Apache
// `identifier` takes both) unless what ends a parameter follows them, `,`,
// `)` or a default's `=`: otherwise they are operators on a value
// (`(X as T)`, `(X in xs)`).
static bool parameter_word_follows(TSLexer *lexer, bool no_close_ahead) {
    char word[sizeof "synchronized"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (word_is(word, len, "instanceof")
        || is_reserved_variable_name(word, len)) {
        return false;
    }
    if (!word_is(word, len, "as") && !word_is(word, len, "in")) {
        return true;
    }
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
        return false;
    }
    int32_t c = lexer->lookahead;
    return c == ',' || c == ')' || c == '=';
}

// After a type in parentheses, which `may_name_type` or a dotted name with
// an upper-case last name starts: whether its type arguments, dimensions
// and varargs dots, then a name, follow, which makes the parentheses a
// parameter list. Groovy's trivia may come between, line breaks included,
// as inside parentheses. `dots` counts the dots of `...` the caller has
// read. Only what Apache's `type` has goes on with the type: inside the
// type arguments names, `.`, `,`, `?` and dimensions; outside them a `[`
// that `]` follows and the three dots of `...`. So a subscript (`X[i]`), a
// ternary (`X ? a : b`), a range (`X..Y`) and `&&` (`A < b && C > d`) make
// no parameter. Counts the items it reads in `items`.
static bool parameter_name_follows(
    TSLexer *lexer,
    unsigned *items,
    unsigned dots,
    bool no_close_ahead
) {
    unsigned depth = 0;
    bool dimension = false;
    for (; *items < NAME_ARGUMENT_LOOKAHEAD_ITEMS; (*items)++) {
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        if (depth == 0) {
            // A dimension's `[` is followed by its `]`, and the dots of
            // `...` by one another and then the name.
            if (dimension && c != ']') {
                return false;
            }
            if (dots > 0 && c != '.'
                && (dots != ELLIPSIS_DOTS || !is_identifier_start(c))) {
                return false;
            }
        }
        if (is_identifier_start(c)) {
            if (depth == 0) {
                return parameter_word_follows(lexer, no_close_ahead);
            }
            // Type arguments hold no string.
            if (read_dollar_slashy_open(lexer)) {
                return false;
            }
            continue;
        }
        if (c == '<') {
            depth++;
        } else if (c == '>' && depth > 0) {
            depth--;
        } else if (depth > 0) {
            if (c != '[' && c != ']' && c != '?' && c != '.' && c != ',') {
                return false;
            }
        } else if (c == '[') {
            dimension = true;
        } else if (c == ']' && dimension) {
            dimension = false;
        } else if (c != '.' || ++dots > ELLIPSIS_DOTS) {
            return false;
        }
        advance(lexer);
    }
    return false;
}

// After a name argument, at the `(` that follows it (`foo bar(x)`):
// whether the parentheses hold the arguments of a call of that name, which
// is then all of the first argument, as in Groovy (`foo(bar(x))`). They do
// not when they hold a parenthesis-free call (`foo bar(baz 1)`, Groovy's
// `foo.bar(baz(1))`, also `bar(println` then `List)` on the next line, as
// Groovy's lexer drops line breaks there) or parameters
// (`foo bar(int a) { }`, `foo bar(a, String... b) { }`, a method
// declaration here, which Groovy rejects): an element that is a type, as
// `may_name_type` reads it, followed by a name, or that starts with an
// annotation, `final` or `def`. The elements are read up to the first one
// that is neither a parameter nor a lone name, within the budget of
// `NAME_ARGUMENT_LOOKAHEAD_ITEMS`; at its end they are arguments. A `/` that
// opens no comment is such an element: after the `(` or a `,` Groovy's lexer
// reads it as the start of a slashy string (`foo bar(x, /re/)`).
//
// `commands` is unset where only parameters make the parentheses no
// arguments: on the line after a typed local (`next_line_continues`, §6.21),
// where a parenthesis-free call in parentheses opens a statement of its own
// (`String s` then `(foo 1)`, §6.18) and so does not go on with the name.
static bool call_arguments_follow(
    TSLexer *lexer,
    bool commands,
    bool no_close_ahead
) {
    unsigned items = 0;
    for (bool first = true; items < NAME_ARGUMENT_LOOKAHEAD_ITEMS;
         first = false, items++) {
        advance(lexer);
        bool slash = false;
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, &slash)) {
            return false;
        }
        if (slash) {
            return true;
        }
        if (lexer->lookahead == '@') {
            return false;
        }
        if (!is_identifier_start(lexer->lookahead)) {
            return true;
        }
        bool upper = is_upper_letter(lexer->lookahead);
        char word[sizeof "synchronized"];
        unsigned len = read_word(lexer, word, sizeof word);
        if (word_is(word, len, "final") || word_is(word, len, "def")) {
            return false;
        }
        if (is_value_keyword(word, len, lexer->lookahead)) {
            return true;
        }
        bool accesses = is_member_access_start(lexer->lookahead);
        int32_t last_name = 0;
        bool nested = false;
        bool ends_in_new = false;
        bool read = read_member_accesses(lexer, &last_name, &nested,
                                         &ends_in_new);
        // `String... a`: the accesses stop at the second dot.
        bool varargs = !read && lexer->lookahead == '.';
        if (!read && !varargs) {
            return true;
        }
        bool type = accesses && last_name != 0
            ? !path_takes_names(last_name, ends_in_new)
            : may_name_type(word, len, upper);
        if (type) {
            return !parameter_name_follows(lexer, &items, varargs ? 1 : 0,
                                           no_close_ahead);
        }
        if (varargs) {
            return true;
        }
        if (commands && first
            && first_argument_follows(lexer, false, true, false,
                                      no_close_ahead)) {
            return false;
        }
        if (!skip_same_line_space(lexer, false)) {
            return true;
        }
        if (is_line_break(lexer->lookahead)) {
            return !skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
                || !is_identifier_start(lexer->lookahead);
        }
        if (lexer->lookahead != ',') {
            return true;
        }
    }
    return true;
}

// SPECIFICATION.md §6.17 — after the name of a lambda's one parameter
// without parentheses and the spaces after it: whether `->` follows, also
// after line breaks and comments (Apache `NL* ARROW`). Only a `-`, a line
// break or a `/` may start the arrow or the trivia before it; at any other
// character this returns false and reads nothing. Otherwise it sets `*read`,
// as it consumes input. None of the three starts the first argument of a
// call (`first_argument_follows`), so a value that is no lambda there is no
// call either.
static bool bare_lambda_arrow_follows(
    TSLexer *lexer,
    bool no_close_ahead,
    bool *read
) {
    int32_t c = lexer->lookahead;
    if (c != '-' && c != '/' && !is_line_break(c)) {
        return false;
    }
    *read = true;
    return arrow_follows(lexer, no_close_ahead);
}

// SPECIFICATION.md §6.17 — decides BARE_LAMBDA_START, an empty token before
// the name of a lambda's one parameter without parentheses (`x -> x`),
// where no call that is a value may start: in an argument, the value of a
// map entry and a closure parameter's default. The name is read once, so
// ARRAY_VALUE_START, valid at the same place (`foo(String[].class)`), is
// decided here too. A name that cannot be a parameter's is checked only
// once the arrow is found, so that the other names cost no comparison.
// So is a keyword of §6.16 whose token is valid at the same place (`var`
// in an enum's first constant's arguments, which are also a constructor's
// parameters): only the whole word tells `var` from `v -> v`.
// The caller has skipped whitespace and seen an identifier start.
static bool scan_bare_lambda_start(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool wants_array,
    bool type_may_start,
    bool no_close_ahead
) {
    bool capitalized = is_capital(lexer->lookahead);
    char word[sizeof "synchronized"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (is_contextual_word(word, len)
        && contextual_word_may_start(word[0], valid_symbols)) {
        lexer->mark_end(lexer);
        return scan_contextual_word(lexer, word, len, valid_symbols,
                                    no_close_ahead);
    }
    // A dollar-slashy string names no parameter and is no element type.
    if (opens_dollar_slashy(word, len, lexer)) {
        return false;
    }
    if (lexer->lookahead != '.' && lexer->lookahead != '[') {
        if (!skip_same_line_space(lexer, false)) {
            return false;
        }
        bool read = false;
        if (bare_lambda_arrow_follows(lexer, no_close_ahead, &read)) {
            lexer->result_symbol = BARE_LAMBDA_START;
            return !is_reserved_bare_lambda_name(word, len);
        }
        if (read) {
            return false;
        }
    }
    // `read_element_type_name`, from the end of the name.
    if (!wants_array
        || (capitalized ? !read_nested_type_names(lexer)
                        : !is_primitive_type(word, len))
        || !array_dimensions_follow(lexer, type_may_start, no_close_ahead)) {
        return false;
    }
    lexer->result_symbol = ARRAY_VALUE_START;
    return true;
}

// After a `\` in a string: consumes the character it escapes, a line
// break whole, `\r\n` included, as Groovy's LineEscape takes `\r\n`, `\n`
// or `\r`. Returns false at the end of input.
static bool skip_escaped_character(TSLexer *lexer) {
    if (lexer->eof(lexer)) {
        return false;
    }
    int32_t c = lexer->lookahead;
    advance(lexer);
    if (c == '\r' && lexer->lookahead == '\n') {
        advance(lexer);
    }
    return true;
}

// SPECIFICATION.md §6.14 — whether the quoted string that starts here, the
// first argument of a call that is a value, closes on its line. One that
// does not is still being typed, and taking it for an argument commits the
// parser to a call whose string never closes: in `"${x"`, an interpolation
// that still lacks its `}`, the closing quote of the string around it opened
// the argument, and the `ERROR` took the class or the file around it, where
// the name alone leaves one missing `}`. A triple quote, which may span
// lines, counts as closing, and so does a `${` in a double-quoted string,
// whose interpolation may; a `\` takes the character after it, a line break
// included (§5.10). Reads no further than the line.
static bool string_closes_on_line(TSLexer *lexer) {
    int32_t quote = lexer->lookahead;
    advance(lexer);
    if (lexer->lookahead == quote) {
        // An empty string, or the start of a triple quote.
        return true;
    }
    for (;;) {
        if (lexer->eof(lexer) || is_line_break(lexer->lookahead)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '\\') {
            if (!skip_escaped_character(lexer)) {
                return false;
            }
        } else if (c == quote
                   || (quote == '"' && c == '$' && lexer->lookahead == '{')) {
            return true;
        }
    }
}

// SPECIFICATION.md §6.14 — decides that a parenthesis-free call starts
// here as a value: of a declarator or an assignment
// (`def out = sh script: 'make'`, `def r = obj.method 1`), or in
// parentheses or an interpolation (`(foo 1)`, `"${foo 1}"`): a name and any
// member accesses written without spaces, then on its line the first
// argument (`first_argument_follows`), a name included unless the receiver
// may name a type (`def x = foo bar`, but not `def x = Foo bar`). The
// token is empty and sits before the name, which the in-grammar lexer
// reads, as it reads the argument. Taking the decision at the name, not
// after it, keeps a line break hidden by a comment from joining two
// statements: in `def x = foo` followed by `/* c */ 1` on the next line,
// the token after the comment would no longer see the line break.
//
// §6.15 — the same name may start an array type that is a value
// (`def c = String[].class`), and ARRAY_VALUE_START, which sits at the
// same place, is decided here too: the two need different things after
// the names, `[` and an argument. So is a keyword of §6.16 whose token is
// valid at the same place, once the whole word is read.
//
// The caller has skipped whitespace and seen an identifier start.
static bool scan_value_start(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool wants_command,
    bool wants_bare_lambda,
    bool wants_array,
    bool type_may_start,
    bool no_close_ahead
) {
    lexer->mark_end(lexer);
    if (!wants_command) {
        if (wants_bare_lambda) {
            return scan_bare_lambda_start(lexer, valid_symbols, wants_array,
                                          type_may_start, no_close_ahead);
        }
        if (!read_element_type_name(lexer)
            || !array_dimensions_follow(lexer, type_may_start, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = ARRAY_VALUE_START;
        return true;
    }
    bool capitalized = is_capital(lexer->lookahead);
    bool upper = is_upper_letter(lexer->lookahead);
    char word[sizeof "synchronized"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (is_contextual_word(word, len)
        && contextual_word_may_start(word[0], valid_symbols)) {
        lexer->mark_end(lexer);
        return scan_contextual_word(lexer, word, len, valid_symbols,
                                    no_close_ahead);
    }
    // A dollar-slashy string is the value itself: no receiver, no
    // parameter's name and no element type.
    if (is_value_keyword(word, len, lexer->lookahead)
        || opens_dollar_slashy(word, len, lexer)) {
        return false;
    }
    bool accesses = is_member_access_start(lexer->lookahead);
    if (wants_bare_lambda && !accesses && lexer->lookahead != '[') {
        if (!skip_same_line_space(lexer, false)) {
            return false;
        }
        bool read = false;
        if (bare_lambda_arrow_follows(lexer, no_close_ahead, &read)) {
            lexer->result_symbol = BARE_LAMBDA_START;
            return !is_reserved_bare_lambda_name(word, len);
        }
        if (read) {
            return false;
        }
        // The spaces stay skipped, so an access after them joins the
        // receiver (`def a = foo .bar 1`), as in Groovy, with the name
        // rule of the first name: turning it down moves more trees away
        // from Groovy than it fixes (SPECIFICATION.md §6.14).
    }
    int32_t last_name = 0;
    bool nested = false;
    bool ends_in_new = false;
    if (!read_member_accesses(lexer, &last_name, &nested, &ends_in_new)
        || !skip_same_line_space(lexer, false)) {
        return false;
    }
    if (lexer->lookahead == '[') {
        bool element = capitalized
            ? nested
            : !accesses && is_primitive_type(word, len);
        if (!wants_array || !element
            || !array_dimensions_follow(lexer, type_may_start, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = ARRAY_VALUE_START;
        return true;
    }
    bool names = accesses ? path_takes_names(last_name, ends_in_new)
        : !may_name_type(word, len, upper);
    // A string is a first argument (`first_argument_follows`), but here only
    // one that closes on its line.
    bool string = lexer->lookahead == '"' || lexer->lookahead == '\'';
    if (string ? !string_closes_on_line(lexer)
               : !first_argument_follows(lexer, true, names, false,
                                         no_close_ahead)) {
        return false;
    }
    lexer->result_symbol = COMMAND_VALUE_START;
    return true;
}

// Whether the string that starts here is the name of a method declared
// with a qualified return type (`java.util.List 'foo'() { }`): it closes
// on this line, has no interpolation, and `(` follows it on the line.
// Groovy reads an interpolated name before `(` as a dynamic call.
static bool method_name_follows(TSLexer *lexer) {
    int32_t quote = lexer->lookahead;
    lexer->advance(lexer, false);
    unsigned closing_quotes = 1;
    if (lexer->lookahead == quote) {
        lexer->advance(lexer, false);
        if (lexer->lookahead == quote) {
            lexer->advance(lexer, false);
            closing_quotes = 3;
        } else {
            closing_quotes = 0;
        }
    }
    // Bounded by the line: a line break or the end of input stops it.
    unsigned quotes_seen = 0;
    while (quotes_seen < closing_quotes) {
        int32_t c = lexer->lookahead;
        if (lexer->eof(lexer) || is_line_break(c) || (quote == '"' && c == '$')) {
            return false;
        }
        lexer->advance(lexer, false);
        if (c == '\\') {
            if (lexer->eof(lexer) || is_line_break(lexer->lookahead)) {
                return false;
            }
            lexer->advance(lexer, false);
        }
        quotes_seen = c == quote ? quotes_seen + 1 : 0;
    }
    return skip_same_line_space(lexer, false) && lexer->lookahead == '(';
}

// SPECIFICATION.md §6.14 — decides that a statement is a
// parenthesis-free call whose receiver has member accesses
// (`obj.method 1`, `a?.b.c x: 1`). The token is empty and sits between
// the receiver's first name, already read, and its first access. Nothing
// can come between the two, so no line break or comment is in play
// there; the accesses must follow each other without spaces, and the
// first argument must start on their line. A name is one when the last
// name of the receiver is not capitalized, Apache's rule for a statement
// (`a.b c` calls, `foo.Bar x` declares). The caller has seen a member
// access start.
static bool scan_command_path_start(TSLexer *lexer, bool no_close_ahead) {
    lexer->mark_end(lexer);
    int32_t last_name = 0;
    bool nested = false;
    bool ends_in_new = false;
    if (!read_member_accesses(lexer, &last_name, &nested, &ends_in_new)
        || !skip_same_line_space(lexer, false)) {
        return false;
    }
    bool quoted = lexer->lookahead == '\'' || lexer->lookahead == '"';
    if (!first_argument_follows(lexer, true,
                                path_takes_names(last_name, ends_in_new), true,
                                no_close_ahead)) {
        return false;
    }
    // Groovy declares a method when the last name is capitalized
    // (`java.util.List 'foo'() { }`) and calls one otherwise
    // (`obj.m 'a'() { }` is `obj.m('a'({ }))`).
    bool capitalized = last_name >= 'A' && last_name <= 'Z';
    if (capitalized && quoted && method_name_follows(lexer)) {
        return false;
    }
    lexer->result_symbol = COMMAND_PATH_START;
    return true;
}

// Skips the trivia Groovy's lexer hides inside a declaration header:
// spaces, tabs, line escapes and block comments, plus line breaks and
// line comments when `newlines` is set. Returns false on a `/` that
// opens neither comment kind (a slashy string or division cannot be
// part of a header), unless `slash` is not NULL: that `/` is then
// consumed, `*slash` is set and the result is true. Returns false on a
// `\` that is not a line escape, and on an
// unterminated block comment. `no_close_ahead` says that every block
// comment from here on is unterminated. Finding that out here cannot
// be remembered: the token would have to start at the keyword, which
// is before the position the fact holds from. The comment branch of
// the scan function reaches the same `/*` next and remembers it.
// `crossed_line`, when not NULL, is set once a line break is skipped.
static bool skip_header_trivia(
    TSLexer *lexer,
    bool newlines,
    bool no_close_ahead,
    bool *crossed_line,
    bool *slash
) {
    for (;;) {
        for (;;) {
            if (!skip_same_line_space(lexer, false)) {
                return false;
            }
            if (!newlines || !is_line_break(lexer->lookahead)) {
                break;
            }
            if (crossed_line != NULL) {
                *crossed_line = true;
            }
            advance(lexer);
        }
        if (lexer->lookahead != '/') {
            return true;
        }
        advance(lexer);
        if (lexer->lookahead == '*') {
            if (no_close_ahead) {
                return false;
            }
            advance(lexer);
            // EOF-bounded: an unterminated comment ends the scan here.
            for (;;) {
                if (lexer->eof(lexer)) {
                    return false;
                }
                if (lexer->lookahead == '*') {
                    advance(lexer);
                    if (lexer->lookahead == '/') {
                        advance(lexer);
                        break;
                    }
                } else {
                    advance(lexer);
                }
            }
        } else if (newlines && lexer->lookahead == '/') {
            while (!lexer->eof(lexer) && !is_line_break(lexer->lookahead)) {
                advance(lexer);
            }
        } else if (slash != NULL) {
            *slash = true;
            return true;
        } else {
            return false;
        }
    }
}

// SPECIFICATION.md §6.15 — whether the dimensions of an array type start
// here, after the name: on the name's line `[`, then `]` with Groovy's
// trivia between, as inside brackets its lexer drops line breaks and
// comments. `String[0]` and `a[i]` are subscripts.
//
// Where a type may also start (TYPE_START valid: at a statement, after
// `(` or after `return`, which a statement may follow), `String[] xs`
// and `(String[]) x` keep their reading. There every dimension is read,
// each further `[` on the line of the `]` before it, and what follows
// them must be what no type is: a member access (`.class`, `.&new`) or
// a method reference (`::new`).
static bool array_dimensions_follow(
    TSLexer *lexer,
    bool type_may_start,
    bool no_close_ahead
) {
    do {
        if (!skip_same_line_space(lexer, false) || lexer->lookahead != '[') {
            return false;
        }
        advance(lexer);
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
            || lexer->lookahead != ']') {
            return false;
        }
        if (!type_may_start) {
            return true;
        }
        advance(lexer);
        if (!skip_same_line_space(lexer, false)) {
            return false;
        }
    } while (lexer->lookahead == '[');
    if (lexer->lookahead == ':') {
        advance(lexer);
        return lexer->lookahead == ':';
    }
    if (lexer->lookahead != '.') {
        return false;
    }
    advance(lexer);
    return lexer->lookahead == '&' || is_identifier_start(lexer->lookahead);
}

// Consumes `keyword` and returns whether the input spells it as a whole
// word, with no identifier character after it. On false, input has been
// consumed, so the caller returns false and the runtime rewinds.
static bool consume_keyword(TSLexer *lexer, const char *keyword) {
    for (const char *c = keyword; *c != '\0'; c++) {
        if (lexer->lookahead != *c) {
            return false;
        }
        advance(lexer);
    }
    return !is_identifier_char(lexer->lookahead);
}

// The name of a declaration header, after its keyword. `mark_end` fixes
// the token at the keyword; what is read from here on is look-ahead and
// is not kept. Between the keyword and the name only spaces, tabs, line
// escapes and block comments may appear: `classDeclaration` in
// GroovyParser.g4 has no `nls` there, so the keyword at the end of a
// line is never one. `private Record record` followed by a constructor
// on the next line, and `private String trait` followed by any member,
// stay a field and that member.
static bool scan_header_name(TSLexer *lexer, bool no_close_ahead) {
    lexer->mark_end(lexer);
    if (!is_inline_space(lexer->lookahead) && lexer->lookahead != '/'
        && lexer->lookahead != '\\') {
        return false;
    }
    if (!skip_header_trivia(lexer, false, no_close_ahead, NULL, NULL)) {
        return false;
    }
    if (!is_identifier_start(lexer->lookahead)) {
        return false;
    }
    // `record $/a/$` and `trait $/a/$` call a method with a string.
    return !read_dollar_slashy_open(lexer);
}

// SPECIFICATION.md §6.7 — `record` is a keyword only at the start of
// a record header: `record`, a name on the same line, then `(` with
// Groovy's lexer trivia allowed in between, or `<` straight after the
// name (type parameters; the grammar's `type_parameters` opener is
// `token.immediate` too). Everywhere else the word
// is an identifier (`record.key()`, `private Record record`,
// `public void record()`), as Apache's `identifier` rule allows. The
// grammar cannot make that choice: with `word: $.identifier` the
// keyword wins wherever it has an action, and once
// record_declaration is a class member that is every member start.
//
// The scanner has already consumed `record` when this is called.
// Before `(` the rule has `nls`, so line breaks and line comments are
// skipped there too. A command-style call `record Point(1, 2)`
// matches the pattern and misfires; §6.7 lists the residuals.
static bool scan_record_header(TSLexer *lexer, bool no_close_ahead) {
    if (!scan_header_name(lexer, no_close_ahead)) {
        return false;
    }
    if (lexer->lookahead == '<') {
        return true;
    }
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
        return false;
    }
    return lexer->lookahead == '(';
}

// SPECIFICATION.md §6.13 — `trait` is a keyword only at the start of a
// trait header: as for `record`, a name on the same line, then, with
// line breaks and comments allowed (the rule has `nls` before each
// clause and the body), `<`, `{` or one of `extends`, `implements` and
// `permits`. Everywhere else the word is an identifier, as Apache's
// `identifier` rule allows. A `<` after a space opens type parameters
// in Groovy but not in the grammar, whose opener is `token.immediate`;
// taking the keyword keeps that an `ERROR` rather than a comparison.
// The scanner has already consumed `trait` when this is called.
static bool scan_trait_header(TSLexer *lexer, bool no_close_ahead) {
    if (!scan_header_name(lexer, no_close_ahead)) {
        return false;
    }
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
        return false;
    }
    if (lexer->lookahead == '{' || lexer->lookahead == '<') {
        return true;
    }
    char word[sizeof "implements"];
    unsigned len = read_word(lexer, word, sizeof word);
    return word_is(word, len, "extends") || word_is(word, len, "implements")
        || word_is(word, len, "permits");
}

// The words of `_modifier` in grammar.js, and `default`, which Apache's
// class modifiers add. Apache's member modifiers take all of them, in
// any order, before a nested type declaration.
static bool is_modifier_word(const char *word, unsigned len) {
    return word_is(word, len, "public") || word_is(word, len, "protected")
        || word_is(word, len, "private") || word_is(word, len, "static")
        || word_is(word, len, "abstract") || word_is(word, len, "final")
        || word_is(word, len, "strictfp") || word_is(word, len, "default")
        || word_is(word, len, "synchronized") || word_is(word, len, "native")
        || word_is(word, len, "transient") || word_is(word, len, "volatile");
}

// SPECIFICATION.md §6.13 — `sealed` is a keyword only before `class`,
// `interface` or `trait`, the words the grammar accepts after it.
// Apache's modifiers end in `nls`, so line breaks and comments may come
// between. Everywhere else the word is an identifier. The scanner has
// already consumed `sealed` when this is called.
//
// Groovy's parser also takes `sealed` before another modifier, an
// annotation, `record` or `enum`. The grammar accepts none of them
// after it, so the keyword there only keeps the `ERROR` at the word.
// Read as a name, the word gave a tree with no `ERROR` that dropped the
// modifier, or lost the declaration to an `ERROR` that began at `sealed`.
// The keyword is taken only on the same line: across a line break the
// same words also start the member after a field named `sealed`, which
// the keyword would turn into an `ERROR`.
static bool scan_sealed_target(TSLexer *lexer, bool no_close_ahead) {
    lexer->mark_end(lexer);
    bool crossed_line = false;
    if (!skip_header_trivia(lexer, true, no_close_ahead, &crossed_line, NULL)) {
        return false;
    }
    if (lexer->lookahead == '@') {
        return !crossed_line;
    }
    char word[sizeof "synchronized"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (word_is(word, len, "class") || word_is(word, len, "interface")
        || word_is(word, len, "trait")) {
        return true;
    }
    if (crossed_line) {
        return false;
    }
    if (word_is(word, len, "non") && lexer->lookahead == '-') {
        advance(lexer);
        return consume_keyword(lexer, "sealed");
    }
    return is_modifier_word(word, len) || word_is(word, len, "record")
        || word_is(word, len, "enum");
}

// SPECIFICATION.md §6.16 — whether `instanceof` and a type follow, read
// from here: a name's test. Groovy never takes `instanceof` as a name, so
// `var instanceof T` and `yield instanceof T` test the word before it.
// Where no type follows (`var instanceof = 1`, which Groovy rejects), the
// word before it keeps the reading it had when it was always a keyword.
static bool instanceof_test_follows(TSLexer *lexer, bool no_close_ahead) {
    return consume_keyword(lexer, "instanceof")
        && skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
        && is_identifier_start(lexer->lookahead);
}

// SPECIFICATION.md §6.16 — whether an expression starts here, after
// Groovy's trivia, line breaks and comments included unless `line` is set:
// what `in` needs after it to be a keyword, and `yield`, on its line. The characters are those an
// expression of the grammar can start with, except that `instanceof` or
// `!instanceof` with a type after it tests the word before it. `!`, `+`
// and `-` start one unless they are `!=`, `+=`, `-=` or `->`. `++` and
// `--` start one only when an operand follows them on their line, so
// `in++` and `yield++` update a name, also when `foo()` follows on the
// next line, as in Groovy; they are read in a loop, not by recursion,
// which a long run of them would turn into a deep stack. `()` starts
// none, so `yield()` is a call. A `/` starts a slashy string unless a
// space, `=` or a second `/` (a line comment, read on the line of a `++`)
// follows it.
static bool expression_follows(
    TSLexer *lexer,
    bool line,
    bool no_close_ahead
) {
    bool same_line = line;
    for (;;) {
        bool slash = false;
        if (!skip_header_trivia(lexer, !same_line, no_close_ahead, NULL,
                                &slash)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        if (slash) {
            return !lexer->eof(lexer) && !is_whitespace(c) && c != '='
                && c != '/';
        }
        if (c == 'i') {
            return !instanceof_test_follows(lexer, no_close_ahead);
        }
        if (is_identifier_start(c) || (c >= '0' && c <= '9') || c == '\''
            || c == '"' || c == '[' || c == '{' || c == '~') {
            return true;
        }
        if (c != '(' && c != '!' && c != '+' && c != '-') {
            return false;
        }
        advance(lexer);
        if (c == '(') {
            return !skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
                || lexer->lookahead != ')';
        }
        if (c == '!') {
            // `!instanceof` is one operator only before whitespace, as the
            // grammar's token is (GroovyLexer.g4 `NOT_INSTANCEOF`).
            if (lexer->lookahead == 'i') {
                return !consume_keyword(lexer, "instanceof")
                    || !is_whitespace(lexer->lookahead)
                    || !skip_header_trivia(lexer, true, no_close_ahead, NULL,
                                           NULL)
                    || !is_identifier_start(lexer->lookahead);
            }
            return lexer->lookahead != '=';
        }
        if (lexer->lookahead != c) {
            return lexer->lookahead != '=' && lexer->lookahead != '>';
        }
        advance(lexer);
        same_line = true;
    }
}

// SPECIFICATION.md §6.16 — the names of a multi-assignment declaration
// opened by `var` alone, from its `(`, then `)` and `=`: `var (a, b) = l`.
// `var(a, b)` and `var (a, b).foo()` call a method named `var`. Only what
// a list of names and types holds is read: names, `,`, `*` (a rest
// binding), `:` (a keyed pair), `.`, `<`, `>`, `?`, `[` and `]`, with
// Groovy's trivia. Any other character ends the look-ahead, so it reads
// no `(` and cannot read the same text twice. Line breaks are allowed
// inside the parentheses and before the `=`, as in Groovy.
static bool names_assignment_follows(TSLexer *lexer, bool no_close_ahead) {
    advance(lexer);
    for (;;) {
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        if (is_identifier_start(c)) {
            // `var ($/a/$)` calls a method named `var`.
            if (read_dollar_slashy_open(lexer)) {
                return false;
            }
            continue;
        }
        if (c == ')') {
            break;
        }
        if (c != ',' && c != '*' && c != ':' && c != '.' && c != '<'
            && c != '>' && c != '?' && c != '[' && c != ']') {
            return false;
        }
        advance(lexer);
    }
    advance(lexer);
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
        || lexer->lookahead != '=') {
        return false;
    }
    advance(lexer);
    return lexer->lookahead != '=' && lexer->lookahead != '~';
}

// SPECIFICATION.md §6.16 — `var` declares the name that follows it, on
// the line or a later one (`var x = 1`, `def f(var x)`), as it did when
// the word was always a keyword. `var instanceof T` tests `var`, as in
// Groovy, and a keyword after it that is no modifier and starts no operand
// (`keyword_starts_operand`), such as `else`, leaves it a name (`if (c) var`
// then `else x`); before `new` it stays the keyword, an `ERROR`, as Groovy's
// `var(new Foo())` is no call the grammar takes.
// Right after `return`, `break` or `continue` on their line, where
// JUMP_STATEMENT_END is valid, it is the value or the label (§6.23), as
// Groovy takes no declaration there: `return var` then `x` on the next
// line returns `var`. Before `(` it opens a multi-assignment: where `var`
// alone may open one (VAR_NAMES_KEYWORD valid), only when the names, `)`
// and `=` follow; after `final`, always. Anywhere else the word is a name.
// In a type body a name and `(` after `var` make it a method's return type,
// as at v0.6.0 (`var m() { }`): Groovy 6's `methodDeclaration` takes the
// modifier and its AST builder rejects the method, so the field the keyword
// opens would only be an `ERROR`. A type body is where `var` alone may open
// a multi-assignment but no typed local starts (DECLARATION_START, §6.21);
// no member starts with `(`, so the look-ahead crosses line breaks, as the
// grammar does between a method's name and its parameters.
static bool scan_var(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool no_close_ahead
) {
    if ((valid_symbols[JUMP_STATEMENT_END] && !valid_symbols[ERROR_SENTINEL])
        || !skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
        return false;
    }
    if (is_identifier_start(lexer->lookahead)) {
        char word[sizeof "synchronized"];
        unsigned len = read_word(lexer, word, sizeof word);
        if (word_is(word, len, "instanceof")) {
            if (skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
                && is_identifier_start(lexer->lookahead)) {
                return false;
            }
        } else if (is_value_keyword(word, len, lexer->lookahead)
                   && !is_modifier_word(word, len)
                   && !word_is(word, len, "def")
                   && !keyword_starts_operand(word, len, lexer->lookahead)) {
            return false;
        } else if (valid_symbols[VAR_NAMES_KEYWORD]
                   && !valid_symbols[DECLARATION_START]
                   && skip_header_trivia(lexer, true, no_close_ahead, NULL,
                                         NULL)
                   && lexer->lookahead == '(') {
            return false;
        }
        lexer->result_symbol = VAR_KEYWORD;
        return valid_symbols[VAR_KEYWORD];
    }
    if (lexer->lookahead != '(') {
        return false;
    }
    if (valid_symbols[VAR_NAMES_KEYWORD]) {
        lexer->result_symbol = VAR_NAMES_KEYWORD;
        return names_assignment_follows(lexer, no_close_ahead);
    }
    // After `final` the word before `(` is always the keyword
    // (`final var (a, b) = l`). Where an expression may start as well
    // (BARE_LAMBDA_START valid), as in the arguments of an enum's first
    // constant, which may be a constructor's parameters, the parentheses
    // call a method named `var` (`enum E { A(var(1)) }`).
    lexer->result_symbol = VAR_KEYWORD;
    return valid_symbols[VAR_KEYWORD] && !valid_symbols[BARE_LAMBDA_START];
}

// Whether `word` is one of the contextual keywords of §6.16.
static bool is_contextual_word(const char *word, unsigned len) {
    return word_is(word, len, "var") || word_is(word, len, "yield")
        || word_is(word, len, "in") || word_is(word, len, "as");
}

// Whether `c` may start one of the words of §6.16 whose token is valid.
static inline bool contextual_word_may_start(
    int32_t c,
    const bool *valid_symbols
) {
    switch (c) {
        case 'v':
            return valid_symbols[VAR_KEYWORD]
                || valid_symbols[VAR_NAMES_KEYWORD];
        case 'y':
            return valid_symbols[YIELD_KEYWORD];
        case 'i':
            return valid_symbols[IN_KEYWORD];
        case 'a':
            return valid_symbols[AS_KEYWORD];
        default:
            return false;
    }
}

// Consumes the word that starts here for as long as it can still be
// `var`, `yield`, `in`, `as` or `int`, and returns which of them it is,
// or NULL. The words that start a statement after another one (`if`,
// `it`, `implementation`, `apply`) are turned down at their first or
// second character, and are not read twice.
static const char *read_contextual_word(TSLexer *lexer) {
    const char *word = NULL;
    switch (lexer->lookahead) {
        case 'v':
            word = "var";
            break;
        case 'y':
            word = "yield";
            break;
        case 'i':
            word = "in";
            break;
        case 'a':
            word = "as";
            break;
        default:
            return NULL;
    }
    for (const char *c = word; *c != '\0'; c++) {
        if (lexer->lookahead != *c) {
            return NULL;
        }
        advance(lexer);
    }
    if (word[0] == 'i' && lexer->lookahead == 't') {
        advance(lexer);
        word = "int";
    }
    return is_identifier_char(lexer->lookahead) ? NULL : word;
}

// SPECIFICATION.md §6.16 — decides the keyword for `var`, `yield`, `in`
// or `as`, which the caller has consumed and marked as the token's end.
// Apache's `identifier` rule lists each word, so each is a name wherever
// its construct does not follow: `yield` and `in` need an expression
// after them, `as` a type, `var` a name or a multi-assignment's names.
// Line breaks and comments before those count as nothing, as they did
// when the words were grammar literals. On false the in-grammar lexer
// reads the word as an `identifier`.
static bool scan_contextual_word(
    TSLexer *lexer,
    const char *word,
    unsigned len,
    const bool *valid_symbols,
    bool no_close_ahead
) {
    if (word_is(word, len, "var")) {
        return (valid_symbols[VAR_KEYWORD] || valid_symbols[VAR_NAMES_KEYWORD])
            && scan_var(lexer, valid_symbols, no_close_ahead);
    }
    // Apache's `yieldStatement` is `YIELD expression`, with no `nls`, in
    // 4.0.x to master, so its value starts on the line of `yield`, past
    // spaces, line escapes and block comments: `yield` then `x` on the next
    // line are two names, as in Groovy 3, which has no `yield` statement.
    // The binary `in` takes `nls` after it.
    if (word_is(word, len, "yield")) {
        lexer->result_symbol = YIELD_KEYWORD;
        return valid_symbols[YIELD_KEYWORD]
            && expression_follows(lexer, true, no_close_ahead);
    }
    if (word_is(word, len, "in")) {
        lexer->result_symbol = IN_KEYWORD;
        return valid_symbols[IN_KEYWORD]
            && expression_follows(lexer, false, no_close_ahead);
    }
    if (word_is(word, len, "as")) {
        lexer->result_symbol = AS_KEYWORD;
        return valid_symbols[AS_KEYWORD]
            && skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
            && is_identifier_start(lexer->lookahead);
    }
    return false;
}

// The keywords of grammar.js that a name can spell. After
// COMMAND_STATEMENT_START the parser takes only a name, so the in-grammar
// lexer would read any of them as one: `return new Foo()` would call a
// method named `return`. The contextual words (§6.7, §6.13, §6.16) are
// not keywords there and are decided before this list is read.
// bindings/rust/tests/scanner_keywords.rs checks the words against
// src/grammar.json.
static bool is_grammar_keyword(const char *word, unsigned len) {
    switch (word[0]) {
        case '_':
            return word_is(word, len, "_");
        case 'a':
            return word_is(word, len, "abstract")
                || word_is(word, len, "assert");
        case 'b':
            return word_is(word, len, "break");
        case 'c':
            return word_is(word, len, "case") || word_is(word, len, "catch")
                || word_is(word, len, "class")
                || word_is(word, len, "continue");
        case 'd':
            return word_is(word, len, "def") || word_is(word, len, "default")
                || word_is(word, len, "do");
        case 'e':
            return word_is(word, len, "else") || word_is(word, len, "enum")
                || word_is(word, len, "extends");
        case 'f':
            return word_is(word, len, "false") || word_is(word, len, "final")
                || word_is(word, len, "finally") || word_is(word, len, "for");
        case 'i':
            return word_is(word, len, "if")
                || word_is(word, len, "implements")
                || word_is(word, len, "import")
                || word_is(word, len, "instanceof")
                || word_is(word, len, "interface");
        case 'n':
            return word_is(word, len, "native") || word_is(word, len, "new")
                || word_is(word, len, "null");
        case 'p':
            return word_is(word, len, "package")
                || word_is(word, len, "permits")
                || word_is(word, len, "pipeline")
                || word_is(word, len, "private")
                || word_is(word, len, "protected")
                || word_is(word, len, "public");
        case 'r':
            return word_is(word, len, "return");
        case 's':
            return word_is(word, len, "static")
                || word_is(word, len, "strictfp")
                || word_is(word, len, "super") || word_is(word, len, "switch")
                || word_is(word, len, "synchronized");
        case 't':
            return word_is(word, len, "throw") || word_is(word, len, "throws")
                || word_is(word, len, "transient")
                || word_is(word, len, "true") || word_is(word, len, "try");
        case 'v':
            return word_is(word, len, "volatile");
        case 'w':
            return word_is(word, len, "while");
        default:
            return false;
    }
}

// Whether a word that `read_word` returned, with `next` the character
// after it, is a keyword that cannot be the first name of a value's
// receiver: a keyword of grammar.js that Groovy reserves, among them the
// words that start a value as keywords (`new`, `switch`, `true`, `false`,
// `null`). Groovy calls no method named `return` in `def x = return 1`,
// and reads the braces of an interpolation that holds one as a closure
// with a statement. It takes `super`, `permits`, `pipeline` and `_` as
// names, and `static` before a `.` (Apache `pathExpression`), so
// `def x = super.foo 1` and `def x = static.foo 1` are calls.
static bool is_value_keyword(const char *word, unsigned len, int32_t next) {
    if (word_is(word, len, "static")) {
        return next != '.';
    }
    return is_grammar_keyword(word, len) && !word_is(word, len, "super")
        && !word_is(word, len, "permits") && !word_is(word, len, "pipeline")
        && !word_is(word, len, "_");
}

// SPECIFICATION.md §6.17 — whether a word that `read_word` returned cannot
// name a lambda's parameter: a keyword of grammar.js that Groovy reserves
// (`null`, `new`, `if`; `_`, `permits` and `pipeline` are names), a
// primitive type name or `void`, which Groovy's `variableDeclaratorId`
// takes only to report the mistake. Taken as a parameter, the in-grammar
// lexer would read any of them as a name.
static bool is_reserved_lambda_name(const char *word, unsigned len) {
    if (is_primitive_type(word, len) || word_is(word, len, "void")) {
        return true;
    }
    return is_grammar_keyword(word, len) && !word_is(word, len, "_")
        && !word_is(word, len, "permits") && !word_is(word, len, "pipeline");
}

// The same for the one parameter without parentheses, where `this` is no
// name either; with them a type may come first (`(String this) -> 1`).
static bool is_reserved_bare_lambda_name(const char *word, unsigned len) {
    return is_reserved_lambda_name(word, len) || word_is(word, len, "this");
}

// The relational operators that a name can spell (§6.11).
typedef enum {
    NO_RELATIONAL_WORD,
    AS_WORD,
    IN_WORD,
    INSTANCEOF_WORD,
} RelationalWord;

// Consumes the word that starts here for as long as it can still be `as`,
// `in` or `instanceof`, and returns which of them it is. At most the ten
// characters of `instanceof` and the one after them are read.
static RelationalWord read_relational_word(TSLexer *lexer) {
    if (lexer->lookahead == 'a') {
        return consume_keyword(lexer, "as") ? AS_WORD : NO_RELATIONAL_WORD;
    }
    if (lexer->lookahead != 'i') {
        return NO_RELATIONAL_WORD;
    }
    advance(lexer);
    if (lexer->lookahead != 'n') {
        return NO_RELATIONAL_WORD;
    }
    advance(lexer);
    if (lexer->lookahead == 's') {
        return consume_keyword(lexer, "stanceof") ? INSTANCEOF_WORD
                                                  : NO_RELATIONAL_WORD;
    }
    return is_identifier_char(lexer->lookahead) ? NO_RELATIONAL_WORD
                                                : IN_WORD;
}

// SPECIFICATION.md §6.14 — consumes a number from its first digit: digits,
// letters and `_` (a radix prefix, a type suffix, an exponent's `e`), a `.`
// that a digit follows (`1.5`) and the sign of a decimal exponent (`1e-5`,
// but not the `-` of `0xE-1`). Returns true at a `.` of a decimal number
// that no digit follows, which it consumes: a member access or a range
// (`1.m()`, `1..3`), which the argument goes on with. No caller reads past
// that `.` to the end of the argument, so what a later line opens with is
// not checked there, and the argument takes it as after a name argument
// (`foo 1..3` then `++`, as `foo a..b` then `++`, which Groovy rejects).
static bool skip_number(TSLexer *lexer) {
    bool hex = false;
    int32_t first = lexer->lookahead;
    advance(lexer);
    if (first == '0' && (lexer->lookahead == 'x' || lexer->lookahead == 'X')) {
        hex = true;
    }
    int32_t last = first;
    // EOF-bounded: lookahead is 0 at end of input, which no branch takes.
    for (;;) {
        int32_t c = lexer->lookahead;
        if ((c >= '0' && c <= '9') || (c >= 'a' && c <= 'z')
            || (c >= 'A' && c <= 'Z') || c == '_') {
            last = c;
            advance(lexer);
        } else if (!hex && (c == '+' || c == '-')
                   && (last == 'e' || last == 'E')) {
            last = c;
            advance(lexer);
        } else if (c == '.' && !hex) {
            advance(lexer);
            if (lexer->lookahead < '0' || lexer->lookahead > '9') {
                return true;
            }
            last = '.';
        } else {
            return false;
        }
    }
}

// SPECIFICATION.md §6.14 — after a literal that is the first argument of a
// statement's call, read to its end: whether the argument goes on past it,
// so that the call opens with COMMAND_STATEMENT_START and the argument is a
// whole expression (`println "a" + b`, `timeout 30 * 60`). On the literal's
// line it does unless a `,`, `;` or `}`, the end of input or a lambda's
// parameters in parentheses (`foo "a" (x) -> x`) follow. On a
// later line only what Groovy lets go on with an expression across a line
// break goes on with it: an operator that starts with `.`, `?`, `*`, `/`,
// `%`, `=`, `!`, `<`, `>`, `&`, `|`, `^` or `:` (`::`), a closure's `{`,
// `as`, `in` or `instanceof` (`foo "a"` then `.trim()` or `?: b`); a name, a
// literal, `(`, `[`, `+`, `-` and `~` open the next statement (`foo 1` then
// `-x`). Of the operators let in, `**` opens no line in Groovy; an
// assignment does (`x` then `= 2`), but Groovy assigns to no literal, so it
// rejects `foo "a"` then `= b`, which is a call here. The grammar ends the
// argument before a `!` that negates. Where
// the argument is the literal alone, the grammar's literal form reads the
// same tree without the token, in fewer steps: with the token, code made
// only of `foo "a"` lines took about a quarter longer to parse.
static bool literal_argument_goes_on(TSLexer *lexer, bool no_close_ahead) {
    bool crossed_line = false;
    bool slash = false;
    if (!skip_header_trivia(lexer, true, no_close_ahead, &crossed_line,
                            &slash)) {
        return false;
    }
    if (slash) {
        return true;
    }
    int32_t c = lexer->lookahead;
    if (lexer->eof(lexer) || c == ',' || c == ';' || c == '}') {
        return false;
    }
    if (!crossed_line) {
        // Groovy appends a lambda to the literal as it does a closure
        // (`pathElement`'s `closureOrLambdaExpression`, `"a"((x) -> x)`),
        // which the grammar does not: the argument would take `(x)` as the
        // literal's arguments and leave the `->` an ERROR. The literal's
        // own form keeps the call and a lambda statement, as on the next
        // line.
        if (c == '(') {
            bool closed = false;
            bool in_string = false;
            return !lambda_parameters_follow(lexer, no_close_ahead, &closed,
                                             &in_string);
        }
        return true;
    }
    if (is_identifier_start(c)) {
        return read_relational_word(lexer) != NO_RELATIONAL_WORD;
    }
    return c == '.' || c == '?' || c == '*' || c == '%' || c == '='
        || c == '!' || c == '<' || c == '>' || c == '&' || c == '|'
        || c == '^' || c == ':' || c == '{';
}

// Whether the first argument of a statement's parenthesis-free call that
// opens with COMMAND_STATEMENT_START starts here, after the spaces after
// the receiver: an object or array creation, an array type that is a
// value, such a call in parentheses (`println(foo 1)`), or a number, a
// quoted string, `true`, `false` or `null` that the argument goes on past
// (`literal_argument_goes_on`), which the token makes a whole expression
// (`println "a" + b`); and, when the receiver takes `names`, a name, a
// negation or `~` as `first_argument_follows` takes them in a statement
// (`println x`, `foo !x`). The key of a named argument (`foo a: 1`,
// `foo true: 1`) and a literal that is the whole argument are not: the
// grammar's form takes them. After a receiver that may name a type, a
// string before `(` names a method (`String 'foo'() { }`), as
// `scan_command_path_start` tells for a dotted one, and the grammar's form
// reads it as before. A dollar-slashy string after a receiver that takes
// `names` is a first argument all the same, as it was while its `$` passed
// for a name: the token makes the argument a whole expression
// (`println $/a/$ + b`). Without `names`, a capitalized name is read with
// any nested type names after it, and a lowercase one whole.
static bool statement_argument_follows(
    TSLexer *lexer,
    bool names,
    bool no_close_ahead
) {
    int32_t c = lexer->lookahead;
    if (c == '(') {
        return parenthesized_command_follows(lexer, no_close_ahead);
    }
    if (c >= '0' && c <= '9') {
        return skip_number(lexer)
            || literal_argument_goes_on(lexer, no_close_ahead);
    }
    if (c == '"' || c == '\'') {
        if (!skip_short_string(lexer)
            || (!names && (!skip_same_line_space(lexer, false)
                           || lexer->lookahead == '('))) {
            return false;
        }
        return literal_argument_goes_on(lexer, no_close_ahead);
    }
    if (!names) {
        if (is_capital(c)) {
            return read_element_type_name(lexer)
                && array_dimensions_follow(lexer, true, no_close_ahead);
        }
        char type_word[sizeof "synchronized"];
        unsigned type_len = read_word(lexer, type_word, sizeof type_word);
        if (word_is(type_word, type_len, "new")) {
            return creation_follows(lexer, no_close_ahead);
        }
        if (is_literal_word(type_word, type_len)) {
            return !key_colon_follows(lexer, type_word, type_len)
                && literal_argument_goes_on(lexer, no_close_ahead);
        }
        return is_primitive_type(type_word, type_len)
            && array_dimensions_follow(lexer, true, no_close_ahead);
    }
    if (c == '!') {
        return negation_follows(lexer);
    }
    if (c == '~') {
        return true;
    }
    if (!is_identifier_start(c)) {
        return false;
    }
    char word[sizeof "synchronized"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (word_is(word, len, "new")) {
        return creation_follows(lexer, no_close_ahead);
    }
    if (opens_dollar_slashy(word, len, lexer)) {
        return true;
    }
    if (is_literal_word(word, len)) {
        return !key_colon_follows(lexer, word, len)
            && literal_argument_goes_on(lexer, no_close_ahead);
    }
    int32_t next = lexer->lookahead;
    if (next == '.' || next == '[') {
        return name_may_be_argument(word, len, next)
            && dotted_argument_rest(lexer, no_close_ahead);
    }
    return !key_colon_follows(lexer, word, len)
        && name_may_be_argument(word, len, next)
        && name_argument_rest(lexer, true, false, is_declaring_word(word, len),
                              no_close_ahead);
}

// SPECIFICATION.md §6.21 — whether a word that `read_word` returned cannot
// name a declared variable: Apache's `variableDeclaratorId` is the
// `identifier` of a lambda's one parameter, so the words are those of
// `is_reserved_bare_lambda_name` (`this`, `super`, `int`, `new`); `String this`
// is Groovy's call `String(this)`.
static bool is_reserved_variable_name(const char *word, unsigned len) {
    return is_reserved_bare_lambda_name(word, len);
}

// SPECIFICATION.md §6.21 — the most the look-ahead of a typed declaration
// reads, in items: a name of its list, a word, a bracket or another
// character of its type arguments, and of a name's initializer the items
// `parameters_follow` counts. That default is read to the `->`
// that would make the list closure parameters, so in a nest of closures
// that open with a declaration (`{ T a = { T a = { …`), and on lines of a
// broken list, each declaration reads the rest of the nest; with the bound
// each reads at most this many items, and the parse stays linear. Type
// arguments or a list of names longer than this are read as before, the
// type followed by its names as statements; a default that runs past the
// budget is a declaration's, not a closure parameter's.
static const unsigned DECLARATION_LOOKAHEAD_ITEMS = 256;

// SPECIFICATION.md §6.21 — whether DECLARATION_START is decided in a `for`
// header: the token is valid where a statement starts, after a statement's
// annotations and as a `for` init, and only the last is a place where an
// expression may start (ARRAY_VALUE_START) and no statement may
// (COMMAND_STATEMENT_START). The state is the one after `for (`, in a fresh
// parse and on reparse alike.
static inline bool in_for_header(const bool *valid_symbols) {
    return valid_symbols[DECLARATION_START]
        && !valid_symbols[COMMAND_STATEMENT_START]
        && valid_symbols[ARRAY_VALUE_START];
}

// Skips what the grammar's extras skip on a line: spaces, tabs, line
// escapes, form feeds and vertical tabs, and with `comments` block comments
// as well, which may span lines, as Groovy's lexer hides a line break inside
// one. Returns false at a `\` that starts no line escape and at a block
// comment that does not close (§6.6). With `comments`, a `/` that opens no
// block comment is consumed and sets `*slash`; the next character is then
// the lookahead.
static bool skip_line_extras(
    TSLexer *lexer,
    bool comments,
    bool no_close_ahead,
    bool *slash
) {
    for (;;) {
        if (comments
            ? !skip_header_trivia(lexer, false, no_close_ahead, NULL, slash)
            : !skip_same_line_space(lexer, false)) {
            return false;
        }
        if ((slash != NULL && *slash)
            || (lexer->lookahead != '\f' && lexer->lookahead != '\v')) {
            return true;
        }
        advance(lexer);
    }
}

// The same across lines: Groovy's trivia (whitespace, line breaks, line
// escapes and comments) and the form feeds and vertical tabs of the
// grammar's extras. A `/` that opens no comment is consumed and sets
// `*slash`.
static bool skip_extras(TSLexer *lexer, bool no_close_ahead, bool *slash) {
    for (;;) {
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, slash)) {
            return false;
        }
        if (*slash || (lexer->lookahead != '\f' && lexer->lookahead != '\v')) {
            return true;
        }
        advance(lexer);
    }
}

// Reads type arguments from their `<` to the matching `>` on the line: names
// (`extends` and `super` among them), `,`, `?`, `.`, `[`, `]` and nested
// `<` `>`, with spaces between, as `type_arguments` in grammar.js takes them.
// A `<` opens arguments only directly after a name, the type's own included,
// as the grammar's `<` is `token.immediate`: `Foo<<Baz>> x` is a shift.
// Counts what it reads in `*items`. Returns false on anything else, at a line
// break and at the end of the budget.
static bool skip_type_arguments(TSLexer *lexer, unsigned *items) {
    unsigned depth = 0;
    // Whether a name ends right before the lookahead.
    bool after_name = true;
    for (; *items < DECLARATION_LOOKAHEAD_ITEMS; (*items)++) {
        int32_t c = lexer->lookahead;
        if (is_inline_space(c) || c == '\\' || c == '\f' || c == '\v') {
            if (!skip_line_extras(lexer, false, false, NULL)) {
                return false;
            }
            after_name = false;
            continue;
        }
        if (is_identifier_start(c)) {
            read_word(lexer, NULL, 0);
            after_name = true;
            continue;
        }
        if (c == '<') {
            if (!after_name) {
                return false;
            }
            depth++;
        } else if (c == '>') {
            if (depth == 0) {
                return false;
            }
            depth--;
            if (depth == 0) {
                advance(lexer);
                return true;
            }
        } else if (c != ',' && c != '?' && c != '.' && c != '[' && c != ']') {
            return false;
        }
        advance(lexer);
        after_name = false;
    }
    return false;
}

// Consumes the dimensions of an array type from the first `[`, as
// `array_dimensions_follow` reads them where a type may start: `[`, Groovy's
// trivia, `]`, and each further `[` on the line of the `]` before it.
// Returns false at a `[` that `]` does not follow.
static bool read_dimensions(TSLexer *lexer, bool no_close_ahead) {
    do {
        advance(lexer);
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)
            || lexer->lookahead != ']') {
            return false;
        }
        advance(lexer);
        if (!skip_same_line_space(lexer, false)) {
            return false;
        }
    } while (lexer->lookahead == '[');
    return true;
}

// After a declared name, a line break and Groovy's trivia, at the first
// character of the next line, which `slash` says is the one after a `/` that
// opens no comment, and which is no `=` (the caller reads an initializer
// there): whether the line goes on with the statement, as Groovy reads it,
// rather than starting the next one. Groovy ends a declaration at the line
// break where a statement can follow, and reads the line before as a call
// otherwise (`String s` then `.trim()` is `String(s.trim())`), which is left
// as before: the type and the name as statements. A line goes on when it
// starts with what no statement starts with: an operator other than `+`,
// `-`, `++`, `--`, `!` and `~` (`.`, `?.`, `*`, `<`, `-=`, `!=`, `/=`, the
// `->` of closure parameters), `instanceof`, `:`, `,` or a closing bracket.
// After a declaration (`declaration`) a `<` that opens type parameters
// starts a generic method instead (`String s` then `<T> T m(T t) { }`,
// `type_parameters_follow`); after a list's element it compares, as before;
// a `/` that opens no comment unless the scanner opens a slashy string
// there, as Groovy's lexer reads one, which needs it to close on its line
// and to start with no `=` (`slashy_body_follows`; `Foo foo` then `/ 2` is
// Groovy's `Foo(foo / 2)`; Groovy reads `/= 2 /` as a slashy string); and
// `(` when the parentheses hold parameters (`call_arguments_follow`):
// `Tab x` then `(int x) { }` stays the method `x`, where Groovy reads the
// declaration and the call `int(x, { })`, which has no reading here. `in`
// and `as` start the next statement, as in Groovy (`InputStream in` then
// `in.close()`).
static bool next_line_continues(
    TSLexer *lexer,
    bool slash,
    bool declaration,
    bool no_close_ahead
) {
    int32_t c = lexer->lookahead;
    if (slash) {
        return c == '=' || !slashy_body_follows(lexer, false);
    }
    if (lexer->eof(lexer)) {
        return false;
    }
    switch (c) {
        case '.':
        case '?':
        case '*':
        case '%':
        case '^':
        case '|':
        case '&':
        case '>':
        case ':':
        case ',':
        case ')':
        case ']':
            return true;
        case '<':
            return !(declaration && type_parameters_follow(lexer));
        case '!':
            return !negation_follows(lexer);
        case '+':
        case '-':
            advance(lexer);
            return lexer->lookahead == '='
                || (c == '-' && lexer->lookahead == '>');
        case '(':
            return !call_arguments_follow(lexer, false, no_close_ahead);
        default:
            break;
    }
    if (!is_identifier_start(c)) {
        return false;
    }
    char word[sizeof "instanceof"];
    unsigned len = read_word(lexer, word, sizeof word);
    return word_is(word, len, "instanceof");
}

// After the first declared name of a typed declaration and the spaces after
// it (`String s`): whether the declaration ends or goes on as a declaration.
// It ends on the line, past spaces and block comments, at `;`, `}`, `else`,
// the end of input or a line comment, or at a line break where the next line
// does not go on with it (`next_line_continues`). Or a list of names follows,
// each after a `,` and Groovy's trivia (`String a, b`), which ends there as
// well. Or the `=` of a name's initializer follows, on its line or the next
// (`String s = 1`, `String a, b = 1`), unless closure parameters may start
// here (CLOSURE_PARAMETERS_START valid) and `->` ends the list
// (`{ String a = 1 -> }`), which makes them closure parameters
// (`parameters_follow`); the parser then reads the initializer. Elsewhere a
// `->` there is a lambda's in the initializer (`final a` then
// `String b = x -> y`). `->`, `(`, a name or anything else on the line is no
// declaration.
//
// In a `for` header (`in_for_header`) Groovy's lexer drops line breaks, so
// they end nothing there: past Groovy's trivia only `;` ends the declaration
// (`for (int i` then `; i < n; i++)`), and `,` and `=` go on with it, while
// `:` and `in` make the loop variable of a for-in loop, and any other text
// is no declaration. A header that is still being typed (`for (int i` then
// the next statement) is then read as before, and error recovery keeps the
// statements around it.
static bool declaration_end_follows(
    TSLexer *lexer,
    unsigned *items,
    const bool *valid_symbols,
    bool no_close_ahead
) {
    bool header = in_for_header(valid_symbols);
    for (; *items < DECLARATION_LOOKAHEAD_ITEMS; (*items)++) {
        bool slash = false;
        if (header) {
            if (!skip_extras(lexer, no_close_ahead, &slash) || slash) {
                return false;
            }
        } else if (!skip_line_extras(lexer, true, no_close_ahead, &slash)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        if (header) {
            if (c == ';') {
                return true;
            }
            if (c != '=' && c != ',') {
                return false;
            }
        } else if (slash) {
            if (c != '/') {
                return false;
            }
            while (!lexer->eof(lexer) && !is_line_break(lexer->lookahead)) {
                advance(lexer);
            }
            c = lexer->lookahead;
        }
        if (lexer->eof(lexer) || c == ';' || c == '}') {
            return true;
        }
        if (is_line_break(c)) {
            slash = false;
            if (!skip_extras(lexer, no_close_ahead, &slash)) {
                return false;
            }
            if (slash || lexer->lookahead != '=') {
                return !next_line_continues(lexer, slash, true,
                                            no_close_ahead);
            }
            c = '=';
        }
        if (c == '=') {
            advance(lexer);
            return lexer->lookahead != '='
                && !(valid_symbols[CLOSURE_PARAMETERS_START]
                     && parameters_follow(lexer,
                                          DECLARATION_LOOKAHEAD_ITEMS - *items,
                                          no_close_ahead));
        }
        if (is_identifier_start(c)) {
            char word[sizeof "else"];
            unsigned len = read_word(lexer, word, sizeof word);
            return word_is(word, len, "else");
        }
        if (c != ',') {
            return false;
        }
        advance(lexer);
        slash = false;
        if (!skip_extras(lexer, no_close_ahead, &slash) || slash
            || !is_identifier_start(lexer->lookahead)) {
            return false;
        }
        char name[sizeof "synchronized"];
        unsigned len = read_word(lexer, name, sizeof name);
        if (is_reserved_variable_name(name, len)
            || opens_dollar_slashy(name, len, lexer)) {
            return false;
        }
    }
    return false;
}

// After the type of a declaration: whether a name follows on the line, no
// keyword Groovy reserves, and then the end of a declaration or its
// initializer (`declaration_end_follows`, which `valid_symbols` passes to).
// Between the type and the name the extras of a line are skipped, block
// comments among them, which may span lines: Groovy's lexer hides them and
// the line breaks in them (`String /* c */ s`). A line comment ends the line.
static bool declared_name_follows(
    TSLexer *lexer,
    unsigned *items,
    const bool *valid_symbols,
    bool no_close_ahead
) {
    bool slash = false;
    if (!skip_line_extras(lexer, true, no_close_ahead, &slash) || slash
        || !is_identifier_start(lexer->lookahead)) {
        return false;
    }
    char name[sizeof "synchronized"];
    unsigned len = read_word(lexer, name, sizeof name);
    return !is_reserved_variable_name(name, len)
        && !opens_dollar_slashy(name, len, lexer)
        && declaration_end_follows(lexer, items, valid_symbols, no_close_ahead);
}

// SPECIFICATION.md §6.21 — after the first name of a statement and type
// arguments directly after it (`List<String> xs`, `foo<T> x`), which
// Apache's isInvalidLocalVariableDeclaration takes for a declaration's type
// whatever its spelling: the arguments, any dimensions and a declared name
// with no initializer.
static bool generic_declaration_follows(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool no_close_ahead
) {
    unsigned items = 0;
    if (!skip_type_arguments(lexer, &items)
        || !skip_same_line_space(lexer, false)) {
        return false;
    }
    if (lexer->lookahead == '[' && !read_dimensions(lexer, no_close_ahead)) {
        return false;
    }
    return declared_name_follows(lexer, &items, valid_symbols, no_close_ahead);
}

// SPECIFICATION.md §6.15, §6.21 — after the first name of a statement or its
// dotted path, at the `[` of dimensions (`String[] xs`, `String[].class`):
// with `element`, the name may be the element type of an array type value,
// which a member access or `::` follows (ARRAY_VALUE_START); otherwise a
// declared name with no initializer may follow, whatever the name's
// spelling (`foo[] x`), as Apache's isInvalidLocalVariableDeclaration takes
// `[` after it for a type (DECLARATION_START). Where no type may start, the
// array type value is decided after the first `]`, as
// `array_dimensions_follow` does, and there is no declaration.
static bool scan_dimensions_start(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool element,
    bool type_may_start,
    bool no_close_ahead
) {
    lexer->result_symbol = ARRAY_VALUE_START;
    if (!type_may_start) {
        return element
            && array_dimensions_follow(lexer, false, no_close_ahead);
    }
    if (!read_dimensions(lexer, no_close_ahead)) {
        return false;
    }
    int32_t c = lexer->lookahead;
    if (c == '.' || c == ':') {
        advance(lexer);
        return element
            && (c == ':' ? lexer->lookahead == ':'
                : lexer->lookahead == '&'
                    || is_identifier_start(lexer->lookahead));
    }
    unsigned items = 0;
    lexer->result_symbol = DECLARATION_START;
    return valid_symbols[DECLARATION_START]
        && declared_name_follows(lexer, &items, valid_symbols,
                                 no_close_ahead);
}

// SPECIFICATION.md §6.14, §6.15, §6.21 — at a `.` after the first name of a
// statement: a dotted name, read to its last name with no space. Its last
// name capitalized, or type arguments or dimensions after it, make it the
// type of a declaration when a declared name with no initializer follows
// (`foo.Bar q`, `java.util.List<String> l`), as Apache's
// isInvalidLocalVariableDeclaration reads it. When every name is
// capitalized it may be the element type of an array type value
// (`Map.Entry[].class`). A dotted receiver of a call is decided after the
// first name (COMMAND_PATH_START), so this returns false for one.
// `capitalized` and `upper` describe the first name.
static bool scan_dotted_statement_start(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool capitalized,
    bool upper,
    bool wants_array,
    bool type_may_start,
    bool no_close_ahead
) {
    bool nested = capitalized;
    bool last_upper = upper;
    while (lexer->lookahead == '.') {
        advance(lexer);
        if (!is_identifier_start(lexer->lookahead)) {
            return false;
        }
        last_upper = is_upper_letter(lexer->lookahead);
        nested = nested && is_capital(lexer->lookahead);
        read_word(lexer, NULL, 0);
    }
    if (lexer->lookahead == '<') {
        lexer->result_symbol = DECLARATION_START;
        return valid_symbols[DECLARATION_START]
            && generic_declaration_follows(lexer, valid_symbols,
                                           no_close_ahead);
    }
    if (!skip_same_line_space(lexer, false)) {
        return false;
    }
    if (lexer->lookahead == '[') {
        return scan_dimensions_start(lexer, valid_symbols,
                                     wants_array && nested, type_may_start,
                                     no_close_ahead);
    }
    unsigned items = 0;
    lexer->result_symbol = DECLARATION_START;
    return last_upper && valid_symbols[DECLARATION_START]
        && declared_name_follows(lexer, &items, valid_symbols,
                                 no_close_ahead);
}

// SPECIFICATION.md §6.14, §6.21 — after the first name of a statement that
// may name a type by Apache's isInvalidLocalVariableDeclaration, a
// capitalized or a primitive type name, and the spaces after it, at a name:
// a call whose first argument starts with `new`, is an array type value or
// is `true`, `false` or `null` that the argument goes on past
// (`Foo new Bar()`, `Foo String[].class`, `Foo true + 1`,
// COMMAND_STATEMENT_START), or a declaration (`String s`,
// `String s = 'a'`, DECLARATION_START). Such a
// receiver takes no other name as an argument (`may_name_type`), so each
// statement that starts with two names is a call, a declaration or neither,
// and never both.
static bool scan_typed_statement_start(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool no_close_ahead
) {
    bool capitalized = is_capital(lexer->lookahead);
    char name[sizeof "synchronized"];
    unsigned len = read_word(lexer, name, sizeof name);
    if (word_is(name, len, "new")) {
        lexer->result_symbol = COMMAND_STATEMENT_START;
        return valid_symbols[COMMAND_STATEMENT_START]
            && creation_follows(lexer, no_close_ahead);
    }
    // `String $/a/$` is a call with a string, which the grammar's form takes.
    if (opens_dollar_slashy(name, len, lexer)) {
        return false;
    }
    if (is_literal_word(name, len)) {
        lexer->result_symbol = COMMAND_STATEMENT_START;
        return valid_symbols[COMMAND_STATEMENT_START]
            && !key_colon_follows(lexer, name, len)
            && literal_argument_goes_on(lexer, no_close_ahead);
    }
    bool keyword = is_reserved_variable_name(name, len);
    if (lexer->lookahead == '.') {
        // `read_element_type_name`, from the end of the name.
        lexer->result_symbol = COMMAND_STATEMENT_START;
        return capitalized && valid_symbols[COMMAND_STATEMENT_START]
            && read_nested_type_names(lexer)
            && array_dimensions_follow(lexer, true, no_close_ahead);
    }
    if (!skip_same_line_space(lexer, false)) {
        return false;
    }
    if (lexer->lookahead == '[') {
        lexer->result_symbol = COMMAND_STATEMENT_START;
        return (capitalized || is_primitive_type(name, len))
            && valid_symbols[COMMAND_STATEMENT_START]
            && array_dimensions_follow(lexer, true, no_close_ahead);
    }
    unsigned items = 0;
    lexer->result_symbol = DECLARATION_START;
    return !keyword && valid_symbols[DECLARATION_START]
        && declaration_end_follows(lexer, &items, valid_symbols,
                                   no_close_ahead);
}

// What `read_closure_call` found after a statement's first name.
typedef enum {
    // No such call: the statement is read as before.
    NO_CLOSURE_CALL,
    // A call with a closure that an expression's member access goes on with:
    // CLOSURE_CALL_START.
    CLOSURE_CALL,
    // The same call as the start of the receiver of a parenthesis-free call:
    // CLOSURE_COMMAND_START.
    CLOSURE_COMMAND,
} ClosureCall;

// Defined after BoundedLexer, which it reads through.
static ClosureCall read_closure_call(TSLexer *lexer, bool no_close_ahead);

// SPECIFICATION.md §6.24 — at the `{` after a statement's first name, which
// the caller has read with the spaces after it. The token is empty, before
// the name.
static bool scan_closure_call_start(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool no_close_ahead
) {
    // After `return`, `break` or `continue` on its line the name is the
    // value or the label, as before `new` (§5.5 item 2): `return` takes the
    // call as an expression without the token.
    if (valid_symbols[JUMP_STATEMENT_END]) {
        return false;
    }
    switch (read_closure_call(lexer, no_close_ahead)) {
        case CLOSURE_CALL:
            lexer->result_symbol = CLOSURE_CALL_START;
            return valid_symbols[CLOSURE_CALL_START];
        case CLOSURE_COMMAND:
            lexer->result_symbol = CLOSURE_COMMAND_START;
            return valid_symbols[CLOSURE_COMMAND_START];
        default:
            return false;
    }
}

// SPECIFICATION.md §6.14 — decides at the first name of a statement,
// before it, whether the statement is a parenthesis-free call whose
// receiver is that name and whose first argument starts with `new`, is
// an array type that is a value (`foo new Bar()`, `foo String[].class`), is
// a name (`println x`) or is a literal that the argument goes on past
// (`println "a" + b`), on the receiver's line. A name is one only after
// a receiver that may not name a type (`may_name_type`) and is no
// contextual word. A key argument and a literal that is the whole argument
// are matched in the grammar instead (§5.5 item 3), and a dotted receiver by
// COMMAND_PATH_START. The token is empty and the in-grammar lexer reads
// the name. Deciding here rather than after the name keeps a comment
// that opens the next line from hiding the line break: after `foo` and
// `/* c */` on the next line the scanner no longer sees one.
//
// The same name may be a contextual keyword (§6.7, §6.13, §6.16) or start
// an array type that is a value (§6.15), whose tokens are valid at a
// statement start as well. The scanner cannot read the name twice, so
// each is decided here, as the branches of the scan function decide it
// where no statement may start.
//
// §6.21 — the same name may also be the type of a declaration without an
// initializer (DECLARATION_START), which is valid after a statement's
// annotations and as a `for` init too, where no call may start. Both
// tokens come from the one reading of the name below, by Apache's
// isInvalidLocalVariableDeclaration on its spelling: a receiver that may
// name a type takes no name argument, and only such a receiver, or a name
// that type arguments or dimensions follow, is a declaration's type. A
// statement that starts with two names is therefore a call, a
// declaration or neither, in every state. Each token is emitted only where
// it is valid; where the other one is the answer, the scan returns false.
//
// The caller has skipped whitespace and seen an identifier start.
static bool scan_statement_start(
    TSLexer *lexer,
    const bool *valid_symbols,
    bool wants_array,
    bool type_may_start,
    bool no_close_ahead
) {
    lexer->mark_end(lexer);
    bool capitalized = is_capital(lexer->lookahead);
    bool upper = is_upper_letter(lexer->lookahead);
    char word[sizeof "synchronized"];
    unsigned len = read_word(lexer, word, sizeof word);
    if (word_is(word, len, "record") && valid_symbols[RECORD_KEYWORD]) {
        lexer->result_symbol = RECORD_KEYWORD;
        return scan_record_header(lexer, no_close_ahead);
    }
    if (word_is(word, len, "trait") && valid_symbols[TRAIT_KEYWORD]) {
        lexer->result_symbol = TRAIT_KEYWORD;
        return scan_trait_header(lexer, no_close_ahead);
    }
    if (word_is(word, len, "sealed") && valid_symbols[SEALED_KEYWORD]) {
        lexer->result_symbol = SEALED_KEYWORD;
        return scan_sealed_target(lexer, no_close_ahead);
    }
    // `in` and `as` are operators only after an expression, where their
    // tokens are valid, and where their operand follows (§6.16): on false
    // the word is a name. A reparse that reuses the statement before lexes
    // the word in the state after that statement, where the operator is not
    // valid, and reads on below. So that both states answer alike, a word
    // that `.` or `<` follows, which starts no operand, is read on below in
    // both (`x = 1` then `in.gov.Foo y` declares `y`), and `as` before
    // anything else that is no name is no receiver in either (`as_word`).
    bool as_word = word_is(word, len, "as");
    bool operator_word = as_word || word_is(word, len, "in");
    bool no_operand = lexer->lookahead == '.' || lexer->lookahead == '<';
    if (is_contextual_word(word, len)
        && contextual_word_may_start(word[0], valid_symbols)
        && !(operator_word && no_operand)) {
        lexer->mark_end(lexer);
        return scan_contextual_word(lexer, word, len, valid_symbols,
                                    no_close_ahead);
    }
    // A keyword is no receiver or element type of an array type, so the
    // look-ahead past one would decide nothing, except before a call in
    // parentheses for the keywords Groovy takes as names: `super(foo 1)` in
    // a constructor calls, as `this(foo 1)` does. Before `new` or an array
    // type value they stay the name and then the argument.
    if (is_grammar_keyword(word, len)) {
        if (!valid_symbols[COMMAND_STATEMENT_START]
            || is_value_keyword(word, len, lexer->lookahead)
            || !skip_same_line_space(lexer, false) || lexer->lookahead != '('
            || !parenthesized_command_follows(lexer, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = COMMAND_STATEMENT_START;
        return true;
    }
    // §6.21 — a declaration's type, by Apache's isInvalidLocalVariableDeclaration
    // on the spelling: a dotted name, which is decided apart, a name that type
    // arguments or dimensions follow, or a capitalized or a primitive type
    // name that a name follows. `this` starts no type, as Groovy's
    // `classOrInterfaceType` takes no keyword: `this.Foo x` is the call
    // `this.Foo(x)` (`super` is a keyword above).
    bool type_word = !word_is(word, len, "this");
    if (lexer->lookahead == '.') {
        return type_word
            && scan_dotted_statement_start(lexer, valid_symbols, capitalized,
                                           upper, wants_array, type_may_start,
                                           no_close_ahead);
    }
    if (lexer->lookahead == '<' && type_word) {
        lexer->result_symbol = DECLARATION_START;
        return valid_symbols[DECLARATION_START]
            && generic_declaration_follows(lexer, valid_symbols,
                                           no_close_ahead);
    }
    // After an expression `as` is the operator before a name, past Groovy's
    // trivia, and a name before anything else, with no token (above): a
    // token here made `x = 1` then `as { }.bar 1` a call on a reparse and
    // an `ERROR` from scratch. An expression is the operand of `in`, and
    // each first argument below starts one, so `in` needs no such rule.
    if (!skip_same_line_space(lexer, false)
        || (as_word && !is_identifier_start(lexer->lookahead))) {
        return false;
    }
    // §6.24 — a call with a closure that a member access goes on with.
    if (lexer->lookahead == '{') {
        return scan_closure_call_start(lexer, valid_symbols, no_close_ahead);
    }
    if (lexer->lookahead == '[') {
        return type_word
            && scan_dimensions_start(
                lexer, valid_symbols,
                wants_array && (capitalized || is_primitive_type(word, len)),
                type_may_start, no_close_ahead);
    }
    if (upper || is_primitive_type(word, len)) {
        if (is_identifier_start(lexer->lookahead)) {
            return scan_typed_statement_start(lexer, valid_symbols,
                                              no_close_ahead);
        }
        // A comment, a form feed or a vertical tab before the name: only a
        // declaration reads past one, as a call's first argument does not
        // (§6.14).
        int32_t c = lexer->lookahead;
        if (c == '/' || c == '\f' || c == '\v') {
            unsigned items = 0;
            lexer->result_symbol = DECLARATION_START;
            return valid_symbols[DECLARATION_START]
                && declared_name_follows(lexer, &items, valid_symbols,
                                         no_close_ahead);
        }
    }
    // `var`, `yield`, `in` or `as` whose own token was not valid here takes
    // no name either: whether that token is valid depends on the token
    // before it (`in` and `as` follow an expression), which an incremental
    // reparse need not restore, so the reading would too.
    bool names = !may_name_type(word, len, upper)
        && !is_contextual_word(word, len);
    if (!valid_symbols[COMMAND_STATEMENT_START]
        || !statement_argument_follows(lexer, names, no_close_ahead)) {
        return false;
    }
    lexer->result_symbol = COMMAND_STATEMENT_START;
    return true;
}

// SPECIFICATION.md §6.11 — skips spaces, tabs, line escapes and block
// comments that close on the line, as `read_paren_close` does before the
// word. Returns false where the line ends or something that is no operand
// comes first: a line comment, a block comment that runs past the end of
// the line, or a `/`.
static bool skip_same_line_trivia(TSLexer *lexer) {
    for (;;) {
        if (!skip_same_line_space(lexer, false)) {
            return false;
        }
        if (lexer->lookahead != '/') {
            return true;
        }
        advance(lexer);
        if (lexer->lookahead != '*') {
            return false;
        }
        advance(lexer);
        for (;;) {
            if (lexer->eof(lexer) || is_line_break(lexer->lookahead)) {
                return false;
            }
            if (lexer->lookahead == '*') {
                advance(lexer);
                if (lexer->lookahead == '/') {
                    advance(lexer);
                    break;
                }
            } else {
                advance(lexer);
            }
        }
    }
}

static bool membership_list_follows(TSLexer *lexer);

// SPECIFICATION.md §6.11 — whether `as` or `in`, consumed by the caller,
// takes what follows it on its line: a type for `as`, so a name other than
// `new`, and for `in` a name, a number or a string. Another of the
// operators that a name spells applies to the cast of the name instead
// (`(x) as instanceof T`, `(x) in as T`). A `[` counts for `in` only where
// the brackets hold several elements (`(x) in [1, 2]`,
// `membership_list_follows`): `(byte) in[i]` casts an element of an array
// named `in`. The read stops at the first character after the trivia, after
// the word that starts there, or in those brackets.
static bool keyword_operand_follows(TSLexer *lexer, bool in) {
    if (!skip_same_line_trivia(lexer)) {
        return false;
    }
    int32_t c = lexer->lookahead;
    if (c == '[' && in) {
        return membership_list_follows(lexer);
    }
    if (c == 'a' || c == 'i') {
        return read_relational_word(lexer) == NO_RELATIONAL_WORD;
    }
    if (c == 'n' && !in) {
        return !consume_keyword(lexer, "new");
    }
    return is_identifier_start(c)
        || (in && ((c >= '0' && c <= '9') || c == '\'' || c == '"'));
}

// SPECIFICATION.md §6.11 — whether a keyword of grammar.js that
// `read_word` returned, with `next` the character after it, may start the
// operand of a cast: `new`, `super`, `switch`, `true`, `false`, `null` and
// `static` before a `.` (Apache `pathExpression`), and the words the grammar
// reserves that Groovy takes as names (`_`, `permits`, `pipeline`). The
// others start a statement or a clause, or are modifiers.
static bool keyword_starts_operand(
    const char *word,
    unsigned len,
    int32_t next
) {
    if (word_is(word, len, "static")) {
        return next == '.';
    }
    return word_is(word, len, "new") || word_is(word, len, "super")
        || word_is(word, len, "switch") || is_literal_word(word, len)
        || word_is(word, len, "_") || word_is(word, len, "permits")
        || word_is(word, len, "pipeline");
}

// What a `)` closes, by what follows it on its line (§6.11).
typedef enum {
    // Nothing: a cast whose operand is on a later line ranks below a
    // parenthesized expression.
    PLAIN_CLOSE,
    // A possible operand of a cast, which then outranks a parenthesized
    // expression.
    OPERAND_CLOSE,
    // The same where the operand is a name, which could also start the
    // type of the next member (CAST_WORD_CLOSE).
    NAME_CLOSE,
    // A relational operator that Groovy applies to the parentheses, a
    // division (§6.2) or a keyword that starts no operand
    // (`keyword_starts_operand`): the parentheses close no cast.
    OPERATOR_CLOSE,
} ParenClose;

// SPECIFICATION.md §6.11 — what a `)` closes, read from the first
// character after it on its line: OPERATOR_CLOSE before a relational
// operator that Groovy applies to the parentheses, where a cast's operand
// cannot start, and before a keyword that starts no operand (`else`),
// NAME_CLOSE before a name, and OPERAND_CLOSE before anything else. `instanceof`, `!instanceof` and `!in` are operators wherever
// Groovy's lexer meets them (GroovyLexer.g4 INSTANCEOF, NOT_INSTANCEOF and
// NOT_IN), so `(x) instanceof Foo` tests `(x)` and is no cast of a name
// `instanceof`. `as` and `in` are names too, and `(Foo) in` casts the name
// `in`; they are the operators when what they take follows on the line, as
// Groovy 6.0.0 reads `(x) as Foo` and `(x) in y` (GROOVY-10355). A `-`, `(`,
// `[`, `{` or `.` goes on with the name in Groovy (`(Foo) in.read()`). A
// name is a word that is no keyword of grammar.js, primitive type name or
// `void`, nor the `$` of a dollar-slashy string; the read stops after the
// word, or after what follows `as` or `in`.
static ParenClose read_relational_operator(TSLexer *lexer) {
    if (lexer->lookahead != '!') {
        if (!is_identifier_start(lexer->lookahead)) {
            return OPERAND_CLOSE;
        }
        char word[sizeof "synchronized"];
        unsigned len = read_word(lexer, word, sizeof word);
        if (word_is(word, len, "instanceof")) {
            return OPERATOR_CLOSE;
        }
        if (word_is(word, len, "in") || word_is(word, len, "as")) {
            return keyword_operand_follows(lexer, word[0] == 'i')
                ? OPERATOR_CLOSE
                : OPERAND_CLOSE;
        }
        // A keyword that starts no operand (`if (a) (b) else c`, where the
        // in-grammar lexer would read `else` as the cast's operand, a name)
        // closes no cast either.
        if (is_grammar_keyword(word, len)
            && !keyword_starts_operand(word, len, lexer->lookahead)) {
            return OPERATOR_CLOSE;
        }
        return is_reserved_lambda_name(word, len)
                || opens_dollar_slashy(word, len, lexer)
            ? OPERAND_CLOSE
            : NAME_CLOSE;
    }
    advance(lexer);
    switch (read_relational_word(lexer)) {
        case INSTANCEOF_WORD:
            // Without whitespace after it, `!instanceof` is no operator
            // (GroovyLexer.g4 `NOT_INSTANCEOF`) but `!` and the keyword,
            // which starts no operand either.
            return OPERATOR_CLOSE;
        case IN_WORD: {
            // GroovyLexer.g4 `NOT_IN`, as in the scan of NOT_IN (§6.12).
            int32_t next = lexer->lookahead;
            return is_whitespace(next) || next == '[' || next == '('
                    || next == '{'
                ? OPERATOR_CLOSE
                : OPERAND_CLOSE;
        }
        default:
            return OPERAND_CLOSE;
    }
}

// A view of a lexer whose input ends `left` characters on, and with `line`
// set at the next line break as well. The readers above stop at the end of
// their input, and answer there as for text that ends too soon: a string that
// does not close, a bracket that is never matched. Through the view each stops
// at its bound as well, so a look-ahead made of them reads no further than a
// fixed number of characters, or than the line, whatever it reads. Nothing is
// emitted through the view: its `mark_end` and `log` are never called and do
// nothing.
typedef struct {
    // First, so that a pointer to the view is a pointer to its lexer.
    TSLexer lexer;
    TSLexer *input;
    unsigned left;
    bool line;
} BoundedLexer;

static inline bool bounded_at_end(const BoundedLexer *view) {
    return view->left == 0
        || (view->line && is_line_break(view->input->lookahead));
}

// Sets the view's lookahead from its input's, after the view or its `line`
// changed.
static inline void bounded_refresh(BoundedLexer *view) {
    view->lexer.lookahead = bounded_at_end(view) ? 0 : view->input->lookahead;
}

static void bounded_advance(TSLexer *lexer, bool skip) {
    BoundedLexer *view = (BoundedLexer *)lexer;
    if (bounded_at_end(view)) {
        return;
    }
    view->left--;
    view->input->advance(view->input, skip);
    bounded_refresh(view);
}

static void bounded_mark_end(TSLexer *lexer) {
    (void)lexer;
}

static uint32_t bounded_get_column(TSLexer *lexer) {
    BoundedLexer *view = (BoundedLexer *)lexer;
    return view->input->get_column(view->input);
}

static bool bounded_is_at_included_range_start(const TSLexer *lexer) {
    const BoundedLexer *view = (const BoundedLexer *)lexer;
    return view->input->is_at_included_range_start(view->input);
}

static bool bounded_eof(const TSLexer *lexer) {
    const BoundedLexer *view = (const BoundedLexer *)lexer;
    return bounded_at_end(view) || view->input->eof(view->input);
}

static void bounded_log(const TSLexer *lexer, const char *format, ...) {
    (void)lexer;
    (void)format;
}

static BoundedLexer bounded_lexer(TSLexer *input, unsigned characters) {
    BoundedLexer view = {
        .lexer = {
            .lookahead = characters == 0 ? 0 : input->lookahead,
            .result_symbol = 0,
            .advance = bounded_advance,
            .mark_end = bounded_mark_end,
            .get_column = bounded_get_column,
            .is_at_included_range_start = bounded_is_at_included_range_start,
            .eof = bounded_eof,
            .log = bounded_log,
        },
        .input = input,
        .left = characters,
        .line = false,
    };
    return view;
}

// SPECIFICATION.md §6.14 — the most characters of a quoted string that is
// a statement's first argument, from its opening quote, that
// `statement_argument_follows` reads to learn whether the argument goes on
// past it (`println "a" + b`). A longer one is taken for the whole argument,
// with no token, so that it is not read twice, once here and once by the
// in-grammar lexer: a script's or a build step's long string, often a
// triple-quoted one on several lines (`sh """…"""`), is read once, and
// `foo "<longer>" + b` is the call `foo "<longer>"` and the statement
// `+b` (`docs/divergences-from-spec.md` §25). Of the 266 real files, no
// string that an argument goes on past is longer than 62 characters.
static const unsigned LITERAL_ARGUMENT_LOOKAHEAD_CHARACTERS = 256;

// Reads a quoted string as `skip_string` does, through a BoundedLexer, so
// that it is false as well at the bound.
static bool skip_short_string(TSLexer *lexer) {
    BoundedLexer view =
        bounded_lexer(lexer, LITERAL_ARGUMENT_LOOKAHEAD_CHARACTERS);
    return skip_string(&view.lexer);
}

// SPECIFICATION.md §6.24 — whether a `/` after a word that `read_word`
// returned opens a slashy string, as Groovy's lexer tells
// (GroovyLexer.g4 `isRegexAllowed`, which reads division only after an
// operand's last token): after a keyword, which ends no operand
// (`return /a/`, `assert /a/`, `case`, `throw`, `yield`, `else`, `do`),
// but not after a name, a number, `this`, `super`, `true`, `false` or
// `null`. `in` and `as` are operators after an operand (`a in /x/`) and
// names elsewhere, and a word after a member access is a name
// (`x.return / 2`), as Groovy 6 reads them (`isSlashyForbidden`); so are
// `_`, `permits` and `pipeline`, which the grammar reserves and Groovy
// does not.
static bool slashy_may_follow_word(
    const char *word,
    unsigned len,
    bool after_operand,
    bool member
) {
    if (member) {
        return false;
    }
    if (word_is(word, len, "in") || word_is(word, len, "as")) {
        return after_operand;
    }
    if (word_is(word, len, "yield")) {
        return true;
    }
    return is_grammar_keyword(word, len) && !is_literal_word(word, len)
        && !word_is(word, len, "super") && !word_is(word, len, "_")
        && !word_is(word, len, "permits") && !word_is(word, len, "pipeline");
}

// What the look-aheads that count brackets on Groovy's tokens keep from one
// token to the next, to tell a slashy string from a division at a `/`. Read
// a character at a time, `x++ / 2` opened a string after the second `+`, and
// the `/` after the operator `in` divided (`x in /a,b/`).
typedef struct {
    // Whether an operand may start at the next token, where a `/` opens a
    // slashy string.
    bool operand;
    // Whether the last token was a member access, after which a keyword is
    // a name (`x.in / 2`).
    bool member;
    // Whether the last token was a `!` after an operand, which `in` goes on
    // as the operator `!in` (`x !in /a,b/`).
    bool negation;
} SlashState;

// After a string, a slashy string or `++` and `--`: an operand ended, or
// Groovy's lexer reads it so (REGEX_CHECK_ARRAY lists `++` and `--`).
static inline void slash_state_after_operand(SlashState *state) {
    state->operand = false;
    state->member = false;
    state->negation = false;
}

// After a `/` that divides, or another operator that needs no telling
// apart.
static inline void slash_state_after_operator(SlashState *state) {
    state->operand = true;
    state->member = false;
    state->negation = false;
}

// After a word that `read_word` returned.
static inline void slash_state_after_word(
    SlashState *state,
    const char *word,
    unsigned len
) {
    state->operand = slashy_may_follow_word(
        word, len, !state->operand || state->negation, state->member);
    state->member = false;
    state->negation = false;
}

// After the punctuation character `c`, which the caller consumed, with
// `next` the character after it. Returns whether `next` is the second
// character of `++` or `--`, which the caller consumes too.
static inline bool slash_state_after_punctuation(
    SlashState *state,
    int32_t c,
    int32_t next
) {
    if ((c == '+' || c == '-') && next == c) {
        slash_state_after_operand(state);
        return true;
    }
    state->negation = c == '!' && !state->operand;
    state->operand = c != ')' && c != ']' && c != '}' && c != '.';
    // `::` is an access too (`x::return / 2`), as `.&` and `.@` are.
    state->member = c == '.' || (c == ':' && next == ':')
        || (state->member && (c == '&' || c == '@' || c == ':'));
    return false;
}

// SPECIFICATION.md §6.11 — the most characters, from the `[` after
// `(x) in`, that `membership_list_follows` reads for the `,` after a list's
// first element. A longer first element leaves the reading of a subscript.
static const unsigned MEMBERSHIP_LIST_LOOKAHEAD_CHARACTERS = 256;

// SPECIFICATION.md §6.11 — at the `[` after `(x) in`: whether the brackets
// hold a `,` with no other bracket open, as a list of several elements does
// (`(x) in [1, 2]`), which Groovy 6 tests `(x)` against. A subscript takes
// several indices too (`in[i, j]`), so the grammar reads both the test and
// the cast of an element of an array named `in`, and the cast outranks the
// test; an array named `in` with several indices is the rarer of the two.
// With one element the cast stays (`(byte) in[i]`). The brackets are read
// on Groovy's tokens through a BoundedLexer, as `next_line_paren_ends_cast`
// reads them, line breaks included; where they cannot be read as far as the
// `,` (the bound, a string or a comment that does not close), the answer is
// no.
static bool membership_list_follows(TSLexer *lexer) {
    BoundedLexer view =
        bounded_lexer(lexer, MEMBERSHIP_LIST_LOOKAHEAD_CHARACTERS);
    TSLexer *input = &view.lexer;
    advance(input);
    unsigned depth = 1;
    SlashState state = {.operand = true};
    for (;;) {
        bool slash = false;
        if (!skip_header_trivia(input, true, false, NULL, &slash)
            || input->eof(input)) {
            return false;
        }
        if (slash) {
            if (state.operand && input->lookahead != '=') {
                if (!slashy_body_follows(input, true)) {
                    return false;
                }
                advance(input);
                slash_state_after_operand(&state);
            } else {
                slash_state_after_operator(&state);
            }
            continue;
        }
        int32_t c = input->lookahead;
        if (c == '\'' || c == '"') {
            if (!skip_string(input)) {
                return false;
            }
            slash_state_after_operand(&state);
            continue;
        }
        if (is_identifier_char(c)) {
            char word[sizeof "synchronized"];
            unsigned len = read_word(input, word, sizeof word);
            if (!opens_dollar_slashy(word, len, input)) {
                slash_state_after_word(&state, word, len);
            } else if (skip_dollar_slashy_string(input, NULL)) {
                slash_state_after_operand(&state);
            } else {
                return false;
            }
            continue;
        }
        if (c == ',' && depth == 1) {
            return true;
        }
        advance(input);
        if (slash_state_after_punctuation(&state, c, input->lookahead)) {
            advance(input);
            continue;
        }
        if (c == '(' || c == '[' || c == '{') {
            depth++;
        } else if ((c == ')' || c == ']' || c == '}') && --depth == 0) {
            return false;
        }
    }
}

// SPECIFICATION.md §6.11 — the most characters, from the `(` that opens the
// line after a cast's `)`, that `next_line_paren_ends_cast` reads for the `)`
// or `]` of brackets around both lines. The read runs at every `)` that ends
// a line before such a `(`, so in a run of short parenthesized lines each
// character is read once from each `)` up to this many characters before it.
// A bracket further on goes unseen, and the cast is turned down as where a
// statement may start (`docs/divergences-from-spec.md` §15); a call's
// arguments that a cast wraps onto the next line close within a line or two.
static const unsigned ENCLOSING_BRACKET_LOOKAHEAD_CHARACTERS = 256;

// SPECIFICATION.md §6.11 — at a `)` with nothing after it on its line (the
// lexer is at the line break, the end of the input or the second `/` of a
// line comment): whether it closes no cast, because the next line with
// anything on it opens with `(` and no bracket holds both lines.
//
// Where a line break ends a statement, that `(` opens the next statement or
// a call's arguments (§6.18), and Groovy reads no cast across the line break:
// `castParExpression castOperandExpression` has no `nls`. The cast, ranked
// last, lost there, but kept alive it was reduced again at every such line,
// in time quadratic in the length of a run of parenthesized lines. Inside
// `(…)` and `[…]` Groovy drops the line break and reads the cast
// (`foo((Foo)` then `(x))`). The scanner has no stack of brackets, so it
// reads on from the `(` for a `)` or `]` with none open, which closes
// brackets around both lines: where one comes before a `}` with none open,
// which closes a block, and within ENCLOSING_BRACKET_LOOKAHEAD_CHARACTERS,
// the cast stays. It stays too where `->` follows the first parentheses
// (`(Foo)` then `(x) -> x`): after an expression the lambda look-ahead does
// not run (§6.17), so the parentheses would open a call's arguments and the
// `->` an ERROR. Groovy 3 reads that call, `(Foo)((x) -> x)`, where the cast
// here takes the lambda (`docs/divergences-from-spec.md` §15).
//
// The brackets are counted on Groovy's tokens, read as the other look-aheads
// read them: strings whole (`skip_string`, a triple-quoted one across lines
// and a `${…}` with the strings in it, and `skip_dollar_slashy_string`), a
// slashy string where an operand may start (`slashy_body_follows`), and
// comments as trivia (`skip_header_trivia`). Those after the `(` are read
// through a BoundedLexer, so the bound holds whatever they read; a string or
// a comment that does not close within it ends the read as the bound does.
static bool next_line_paren_ends_cast(TSLexer *lexer, bool no_close_ahead) {
    while (!lexer->eof(lexer) && !is_line_break(lexer->lookahead)) {
        advance(lexer);
    }
    while (!lexer->eof(lexer) && is_whitespace(lexer->lookahead)) {
        advance(lexer);
    }
    if (lexer->lookahead != '(') {
        return false;
    }
    BoundedLexer view =
        bounded_lexer(lexer, ENCLOSING_BRACKET_LOOKAHEAD_CHARACTERS);
    TSLexer *input = &view.lexer;
    unsigned depth = 0;
    bool first_closed = false;
    // An operand may start here, as in `parentheses_open_statement`: a `/`
    // there opens a slashy string.
    SlashState state = {.operand = true};
    for (;;) {
        bool slash = false;
        if (!skip_header_trivia(input, true, no_close_ahead, NULL, &slash)
            || input->eof(input)) {
            return true;
        }
        if (slash) {
            if (state.operand && input->lookahead != '=') {
                if (!slashy_body_follows(input, true)) {
                    return true;
                }
                advance(input);
                slash_state_after_operand(&state);
            } else {
                slash_state_after_operator(&state);
            }
            continue;
        }
        int32_t c = input->lookahead;
        if (c == '\'' || c == '"') {
            if (!skip_string(input)) {
                return true;
            }
            slash_state_after_operand(&state);
            continue;
        }
        if (is_identifier_char(c)) {
            char word[sizeof "synchronized"];
            unsigned len = read_word(input, word, sizeof word);
            if (!opens_dollar_slashy(word, len, input)) {
                slash_state_after_word(&state, word, len);
            } else if (skip_dollar_slashy_string(input, NULL)) {
                slash_state_after_operand(&state);
            } else {
                return true;
            }
            continue;
        }
        advance(input);
        if (slash_state_after_punctuation(&state, c, input->lookahead)) {
            advance(input);
            continue;
        }
        if (c == '(' || c == '[' || c == '{') {
            depth++;
        } else if (c == ')' || c == ']' || c == '}') {
            if (depth == 0) {
                return c == '}';
            }
            depth--;
            if (depth == 0 && !first_closed) {
                first_closed = true;
                if (!skip_same_line_space(input, false)) {
                    return true;
                }
                if (input->lookahead == '-') {
                    advance(input);
                    if (input->lookahead == '>') {
                        return false;
                    }
                    slash_state_after_operator(&state);
                }
            }
        }
    }
}

// SPECIFICATION.md §6.24 — the most characters `read_closure_call` reads from
// the `{` after a statement's first name: the closure, which must close on
// the name's line, what follows its `}` up to the member access, and the
// receiver and first argument of a call that the access starts. The read runs
// at every `name {` that opens a statement and ends at the end of the line,
// so a block whose `{` ends its line costs a character. A line of nested calls
// with closures (`a { a { a { … `) is read from each `{` to the end of its
// braces or of the line, and the bound keeps that to this many characters
// from each `{` rather than the rest of the line: such a line is read about
// a quarter of this many times per character (measured: 35 times at 128, 65
// at 256, 3 before the read). A file of short lines of nested closures
// (`a { b { c { } } }`) is read 4.6 times per character, 2.9 before. A
// closure and its access that span more than this from
// the `{` are read as before; this covers a line of 120 columns, and the
// longest closure that opens and closes a statement's line in the real files
// measured is 66 characters.
static const unsigned CLOSURE_CALL_LOOKAHEAD_CHARACTERS = 128;

// SPECIFICATION.md §6.24 — inside `depth` opening brackets (`{`, `(`, `[`),
// consumes the tokens to the bracket that closes the outermost. They are
// counted on Groovy's tokens, as `next_line_paren_ends_cast` counts them:
// strings whole (`skip_string`, a `${…}` with the strings in it), a slashy
// string where Groovy's lexer opens one (`slashy_body_follows`): after an
// operator, a bracket that opens or a keyword (`slashy_may_follow_word`),
// not after an operand or `++` and `--`; a dollar-slashy string
// (`skip_dollar_slashy_string`) and block comments as trivia. `operand`
// says whether an operand may start at the first token. Returns false where
// the brackets do not close: at the end of the input, which a
// BoundedLexer's bound or line sets, and at a string, a comment or an
// interpolation that does not close. A line comment ends the read as well,
// unless `lines` is set: it is then trivia, and a line break any other
// character.
static bool skip_closure_tokens(
    TSLexer *input,
    unsigned depth,
    bool operand,
    bool lines,
    bool no_close_ahead
) {
    SlashState state = {.operand = operand};
    // Whether the last token was the word `yield`, not after an access.
    bool yield = false;
    while (depth > 0) {
        bool slash = false;
        if (!skip_header_trivia(input, false, no_close_ahead, NULL, &slash)
            || input->eof(input)) {
            return false;
        }
        if (slash) {
            if (input->lookahead == '/') {
                if (!lines) {
                    return false;
                }
                while (!input->eof(input)
                       && !is_line_break(input->lookahead)) {
                    advance(input);
                }
                slash_state_after_operator(&state);
            } else if (state.operand && input->lookahead != '=') {
                if (slashy_body_follows(input, !yield)) {
                    advance(input);
                    slash_state_after_operand(&state);
                } else if (yield && is_whitespace(input->lookahead)) {
                    // No slashy string opens after `yield` here, so the
                    // grammar reads `yield` as a name and the `/` as
                    // division (§6.16): `foo { yield / 2 }.bar()`.
                    slash_state_after_operator(&state);
                } else {
                    return false;
                }
            } else {
                slash_state_after_operator(&state);
            }
            yield = false;
            continue;
        }
        int32_t c = input->lookahead;
        yield = false;
        if (c == '\'' || c == '"') {
            if (!skip_string(input)) {
                return false;
            }
            slash_state_after_operand(&state);
            continue;
        }
        if (is_identifier_char(c)) {
            char word[sizeof "synchronized"];
            unsigned len = read_word(input, word, sizeof word);
            if (opens_dollar_slashy(word, len, input)) {
                if (!skip_dollar_slashy_string(input, NULL)) {
                    return false;
                }
                slash_state_after_operand(&state);
            } else {
                yield = !state.member && word_is(word, len, "yield");
                slash_state_after_word(&state, word, len);
            }
            continue;
        }
        advance(input);
        if (slash_state_after_punctuation(&state, c, input->lookahead)) {
            advance(input);
            continue;
        }
        if (c == '{' || c == '(' || c == '[') {
            depth++;
        } else if (c == '}' || c == ')' || c == ']') {
            depth--;
        }
    }
    return true;
}

// SPECIFICATION.md §6.24 — consumes a member access that may follow the `}`
// of a closure or a name after it: `.` other than `..`, `?.`, `??.`, `*.`,
// `.&`, `.@` and `::`. `*receiver` is cleared at `::`, which
// `_closure_command_path` does not take.
static bool read_closure_access(TSLexer *lexer, bool *receiver) {
    if (lexer->lookahead == ':') {
        advance(lexer);
        if (lexer->lookahead != ':') {
            return false;
        }
        advance(lexer);
        *receiver = false;
        return true;
    }
    bool dot = true;
    return is_member_access_start(lexer->lookahead)
        && read_member_access(lexer, &dot)
        && !(dot && lexer->lookahead == '.');
}

// What follows an access's name, or brackets after it, in
// `read_closure_accesses`.
typedef enum {
    // Another access, consumed: a name follows.
    NEXT_ACCESS,
    // Brackets, consumed.
    BRACKETS,
    // Neither on the line, nothing consumed there: the accesses end.
    ACCESSES_END,
    // Parentheses that hold a parenthesis-free call (`.bar(baz 1)`), the
    // first argument of a call that the accesses are the receiver of.
    PARENTHESIZED_COMMAND,
    // What the read cannot take or tell, such as brackets that do not
    // close within the bound: the accesses are an expression.
    ACCESS_STOP,
} AccessRest;

// SPECIFICATION.md §6.24 — at the `(` after an access's name: whether the
// parentheses hold a parenthesis-free call, read as far as
// `parenthesized_command_follows` reads it (Groovy's `bar(baz 1)`, which
// `first_argument_follows` takes after a receiver), or are read past as
// brackets. Up to the first argument `parenthesized_command_follows`
// consumes no bracket, so where it finds none the brackets are read on from
// there, past `in`, `as` and `instanceof` too, which are operators after the
// name; where another name may start one, the answer is that of
// `first_argument_follows`, and ACCESS_STOP where it is no, which only
// parentheses that the grammar reads with an ERROR in them give.
static AccessRest read_closure_parentheses(
    TSLexer *input,
    bool no_close_ahead
) {
    advance(input);
    bool slash = false;
    if (!skip_header_trivia(input, true, no_close_ahead, NULL, &slash)) {
        return ACCESS_STOP;
    }
    bool operand = true;
    if (slash) {
        // A slashy string opens the parentheses (`.bar(/x/)`). It is read
        // whole and the parentheses on to their `)`, so that an argument
        // after them is found (`foo { }.bar(/x/).baz 1`); stopping here
        // made the accesses a call and that argument a statement.
        if (input->lookahead == '=' || !slashy_body_follows(input, true)) {
            return ACCESS_STOP;
        }
        advance(input);
        operand = false;
    } else if (is_identifier_start(input->lookahead)) {
        bool upper = is_upper_letter(input->lookahead);
        char word[sizeof "synchronized"];
        unsigned len = read_word(input, word, sizeof word);
        if (opens_dollar_slashy(word, len, input)) {
            if (!skip_dollar_slashy_string(input, NULL)) {
                return ACCESS_STOP;
            }
            operand = false;
        } else if (is_value_keyword(word, len, input->lookahead)) {
            operand = slashy_may_follow_word(word, len, false, false);
        } else {
            bool accesses = is_member_access_start(input->lookahead);
            int32_t last_name = 0;
            bool nested = false;
            bool ends_in_new = false;
            if (read_member_accesses(input, &last_name, &nested,
                                     &ends_in_new)) {
                bool names = accesses
                    ? path_takes_names(last_name, ends_in_new)
                    : !may_name_type(word, len, upper);
                if (!skip_same_line_space(input, false)) {
                    return ACCESS_STOP;
                }
                int32_t c = input->lookahead;
                if (is_literal_start(c) || (names && c == '~')) {
                    return PARENTHESIZED_COMMAND;
                }
                if (names && c == '!') {
                    if (negation_follows(input)) {
                        return PARENTHESIZED_COMMAND;
                    }
                } else if (is_identifier_start(c)) {
                    bool capitalized = is_capital(c);
                    char argument[sizeof "synchronized"];
                    unsigned argument_len =
                        read_word(input, argument, sizeof argument);
                    if (!word_is(argument, argument_len, "in")
                        && !word_is(argument, argument_len, "as")
                        && !word_is(argument, argument_len, "instanceof")) {
                        return first_argument_word_follows(
                                   input, argument, argument_len, capitalized,
                                   false, names, false, no_close_ahead)
                            ? PARENTHESIZED_COMMAND
                            : ACCESS_STOP;
                    }
                    // An operator on the name (`.bar(a in b)`), unless a
                    // key (`.bar(a as: 1)`): the parentheses are read on to
                    // their `)`, so that what follows them is told as after
                    // any other brackets.
                    if (key_colon_follows(input, argument, argument_len)) {
                        return PARENTHESIZED_COMMAND;
                    }
                } else {
                    operand = false;
                }
            }
        }
    }
    return skip_closure_tokens(input, 1, operand, true, no_close_ahead)
        ? BRACKETS
        : ACCESS_STOP;
}

// SPECIFICATION.md §6.24 — after an access's name (`after_name`) or the
// brackets after it: a member access, brackets that open on its line
// (`(`, `[`, `?[`, `{`), past the trivia on the line; or, where none follows
// on the line, an access that opens a later one, past line breaks and
// comments. `*bracketed` is set at brackets.
static AccessRest read_closure_access_rest(
    TSLexer *input,
    bool after_name,
    bool *receiver,
    bool *bracketed,
    bool no_close_ahead
) {
    bool slash = false;
    if (!skip_header_trivia(input, false, no_close_ahead, NULL, &slash)
        || (slash && input->lookahead != '/')) {
        return ACCESS_STOP;
    }
    if (slash || input->eof(input) || is_line_break(input->lookahead)) {
        // No argument on the line: only an access that opens a later line
        // goes on with the name.
        while (!input->eof(input) && !is_line_break(input->lookahead)) {
            advance(input);
        }
        return skip_header_trivia(input, true, no_close_ahead, NULL, NULL)
                && read_closure_access(input, receiver)
            ? NEXT_ACCESS
            : ACCESS_STOP;
    }
    int32_t c = input->lookahead;
    if (c == '?') {
        advance(input);
        if (input->lookahead != '[') {
            // `?.` or `??.`, as `read_member_access` reads them.
            if (input->lookahead == '?') {
                advance(input);
            }
            if (input->lookahead != '.') {
                return ACCESS_STOP;
            }
            advance(input);
            return NEXT_ACCESS;
        }
        c = '[';
    }
    if (c == '(' && after_name) {
        AccessRest rest = read_closure_parentheses(input, no_close_ahead);
        *bracketed = *bracketed || rest != PARENTHESIZED_COMMAND;
        return rest;
    }
    if (c == '(' || c == '[' || c == '{') {
        *bracketed = true;
        advance(input);
        return skip_closure_tokens(input, 1, true, true, no_close_ahead)
            ? BRACKETS
            : ACCESS_STOP;
    }
    if (c == ':' || is_member_access_start(c)) {
        return read_closure_access(input, receiver) ? NEXT_ACCESS
                                                    : ACCESS_STOP;
    }
    return ACCESSES_END;
}

// SPECIFICATION.md §6.24 — after the `}` of a statement's closure that closed
// on its line: whether a member access follows, past line breaks and
// comments, which Groovy's lexer hides there (Apache's `pathElement` takes
// `nls` before and after each access), and whether the accesses are the
// receiver of a parenthesis-free call (`foo { }.bar 1` is
// `foo({ }).bar(1)`). They are when each takes a name of the grammar's
// receiver (`_closure_command_path`: no quoted name, no `::`) and a first
// argument that `obj.m 1` takes (`first_argument_follows`, as
// `scan_command_path_start` reads it) follows the last name on its line.
// Where such an argument follows another receiver (`foo { }::bar 1`,
// `foo { }.'bar' 1`), no token is emitted, as no rule reads that call, and
// the statement stays the ERROR it was: the call alone would leave the
// argument a statement of its own. So it does where brackets follow an
// access and a first argument comes after them or after a later access
// (`foo { }.bar().baz 1`, `a { }.b { }.c 1`, `foo { }.bar[0] 1`), which no
// receiver of the grammar takes either (`foo().baz 1` is `foo().baz` and
// `1`, §5.5 item 2), and where a name follows `new` (`foo { }.new X`, as
// `obj.new X` is `obj.new` and `X`). The brackets are read past, line
// breaks included (`skip_closure_tokens`); `*bracketed` is set once the read
// is in them.
static ClosureCall read_closure_accesses(
    TSLexer *input,
    bool no_close_ahead,
    bool *bracketed
) {
    if (!skip_header_trivia(input, true, no_close_ahead, NULL, NULL)) {
        return NO_CLOSURE_CALL;
    }
    bool receiver = true;
    if (!read_closure_access(input, &receiver)) {
        return NO_CLOSURE_CALL;
    }
    bool ends_in_new = false;
    AccessRest rest = NEXT_ACCESS;
    while (rest != ACCESSES_END) {
        if (rest == NEXT_ACCESS) {
            if (!skip_header_trivia(input, true, no_close_ahead, NULL, NULL)) {
                return CLOSURE_CALL;
            }
            if (input->lookahead == '\'' || input->lookahead == '"') {
                if (!skip_string(input)) {
                    return CLOSURE_CALL;
                }
                receiver = false;
                ends_in_new = false;
            } else if (is_identifier_start(input->lookahead)) {
                char name[sizeof "new"];
                ends_in_new =
                    word_is(name, read_word(input, name, sizeof name), "new");
            } else {
                return CLOSURE_CALL;
            }
        } else {
            ends_in_new = false;
        }
        rest = read_closure_access_rest(input, rest == NEXT_ACCESS,
                                        &receiver, bracketed, no_close_ahead);
        if (rest == ACCESS_STOP) {
            return CLOSURE_CALL;
        }
        if (rest == PARENTHESIZED_COMMAND) {
            return receiver && !*bracketed ? CLOSURE_COMMAND
                                           : NO_CLOSURE_CALL;
        }
    }
    if (!skip_same_line_space(input, false)) {
        return CLOSURE_CALL;
    }
    if (ends_in_new && is_identifier_start(input->lookahead)) {
        return NO_CLOSURE_CALL;
    }
    // A receiver that starts with a call names no type, so a capitalized
    // last name takes a name too (`foo { }.Bar x` is `foo({ }).Bar(x)`),
    // where `foo.Bar x` declares `x`. After brackets a name is an argument
    // to Groovy as well (`foo { }.bar() x` is `foo({ }).bar().x`).
    if (!first_argument_follows(input, false, *bracketed || !ends_in_new,
                                true, no_close_ahead)) {
        return CLOSURE_CALL;
    }
    return receiver && !*bracketed ? CLOSURE_COMMAND : NO_CLOSURE_CALL;
}

// SPECIFICATION.md §6.24 — at the `{` after the first name of a statement,
// which the caller has read: whether the braces close on this line and a
// member access follows them (`skip_closure_tokens`,
// `read_closure_accesses`). Groovy reads `foo { }.bar()` as
// `foo({ }).bar()`, where `command_chain` would take `foo { }` as the whole
// statement. The read goes through a BoundedLexer, which ends at the line
// break until the `}`. One that reaches the bound answers NO_CLOSURE_CALL
// whatever it read, so that an argument the bound cuts off never leaves the
// call and a statement of its own; but once it is in brackets after an
// access (`foo { }.each(…)`), whatever comes after them is read as after
// `foo().bar(…)`, as before brackets were read past.
static ClosureCall read_closure_call(TSLexer *lexer, bool no_close_ahead) {
    BoundedLexer view =
        bounded_lexer(lexer, CLOSURE_CALL_LOOKAHEAD_CHARACTERS);
    view.line = true;
    bounded_refresh(&view);
    advance(&view.lexer);
    if (!skip_closure_tokens(&view.lexer, 1, true, false, no_close_ahead)) {
        return NO_CLOSURE_CALL;
    }
    view.line = false;
    bounded_refresh(&view);
    bool bracketed = false;
    ClosureCall call =
        read_closure_accesses(&view.lexer, no_close_ahead, &bracketed);
    return view.left == 0 && !bracketed ? NO_CLOSURE_CALL : call;
}

// SPECIFICATION.md §6.11 — what follows a `)` on its line. The `)` is
// already consumed when this is called.
// `castParExpression castOperandExpression` in GroovyParser.g4 has no
// `nls` between the two, so where a line break can end a statement,
// `x = (a)` followed by `foo()` on the next line is two statements.
// The grammar ranks a cast by this answer; it does not forbid one,
// because inside `(…)` and `[…]` a line break ends nothing. Spaces,
// tabs, line escapes and block comments that close on the line may
// come first. A line comment ends the line, and any other `/` is a
// division (OPERATOR_CLOSE).
static ParenClose read_paren_close(TSLexer *lexer) {
    lexer->mark_end(lexer);
    for (;;) {
        if (!skip_same_line_space(lexer, false)) {
            return OPERAND_CLOSE;
        }
        if (lexer->eof(lexer) || is_line_break(lexer->lookahead)) {
            return PLAIN_CLOSE;
        }
        if (lexer->lookahead != '/') {
            return read_relational_operator(lexer);
        }
        advance(lexer);
        if (lexer->lookahead == '/') {
            return PLAIN_CLOSE;
        }
        // §6.2 — after `)` Groovy's lexer reads `/` as division, never as
        // the start of a slashy string, so no cast's operand starts there
        // (`(a)/2 + (b)/2`).
        if (lexer->lookahead != '*') {
            return OPERATOR_CLOSE;
        }
        // A block comment that closes on this line is skipped, and the
        // answer depends on what follows it. One that runs past the
        // end of the line counts as something on the line: Groovy's
        // lexer drops the comment with its line breaks, and reading
        // on would scan an unterminated comment to the end of input.
        advance(lexer);
        for (;;) {
            if (lexer->eof(lexer) || is_line_break(lexer->lookahead)) {
                return OPERAND_CLOSE;
            }
            if (lexer->lookahead == '*') {
                advance(lexer);
                if (lexer->lookahead == '/') {
                    advance(lexer);
                    break;
                }
            } else {
                advance(lexer);
            }
        }
    }
}

// What follows the `<` after the name of a type, as `read_type_arguments`
// reads it.
typedef enum {
    // Type arguments that close with their `>`.
    ARGUMENTS_CLOSE,
    // Something no type argument holds: the `<` is a comparison's.
    ARGUMENTS_COMPARE,
    // Type arguments longer than the read.
    ARGUMENTS_UNREAD,
} TypeArguments;

// SPECIFICATION.md §3.2.2 — whether the type arguments of a type, whose
// `<` the caller consumed, close with their `>`: names, dots, commas, `?`,
// `[`, `]` and a nested `<` right after a name, with Groovy's trivia, line
// breaks and comments included, between them. Anything else, an operator
// or a literal, makes the `<` a comparison: Groovy reads `x as a.B<c` as
// `(x as a.B) < c`. At most DECLARATION_LOOKAHEAD_ITEMS items are read.
static TypeArguments read_type_arguments(TSLexer *lexer, bool no_close_ahead) {
    unsigned depth = 1;
    // Whether a name ends right before the lookahead.
    bool after_name = false;
    for (unsigned items = 0; items < DECLARATION_LOOKAHEAD_ITEMS; items++) {
        int32_t c = lexer->lookahead;
        if (is_whitespace(c) || c == '\\' || c == '/') {
            bool slash = false;
            if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, &slash)
                || slash) {
                return ARGUMENTS_COMPARE;
            }
            after_name = false;
            continue;
        }
        if (is_identifier_start(c)) {
            // A string is a literal: `x as a.B<$/c/$` compares.
            if (read_dollar_slashy_open(lexer)) {
                return ARGUMENTS_COMPARE;
            }
            after_name = true;
            continue;
        }
        if (c == '<') {
            if (!after_name) {
                return ARGUMENTS_COMPARE;
            }
            depth++;
        } else if (c == '>') {
            depth--;
            if (depth == 0) {
                return ARGUMENTS_CLOSE;
            }
        } else if (c != ',' && c != '?' && c != '.' && c != '[' && c != ']') {
            return ARGUMENTS_COMPARE;
        }
        advance(lexer);
        after_name = false;
    }
    return ARGUMENTS_UNREAD;
}

// SPECIFICATION.md §6.17 — the most the look-ahead for a lambda's
// parameters reads, in items: a word, a string, a comment, an annotation or
// a punctuation character. It runs at every `(` that may start an
// expression, and the `(` of a default value may start one too, so without a
// bound `(a = (a = (a = …` would be read in time quadratic in its depth.
// With it each `(` reads at most this many items, and the parse stays
// linear. A parameter list longer than this is read as no lambda's; real
// ones are a few items.
static const unsigned LAMBDA_PARAMETERS_LOOKAHEAD_ITEMS = 256;

// Counts one item of the look-ahead for a lambda's parameters. Returns false
// once the budget is spent.
static inline bool spend_lambda_item(unsigned *items) {
    if (*items >= LAMBDA_PARAMETERS_LOOKAHEAD_ITEMS) {
        return false;
    }
    (*items)++;
    return true;
}

// Reads Groovy's tokens, counting brackets, up to a `)` with no bracket
// open, and, when `comma` is set, a `,` with none open; it leaves that
// character. That is a parameter's default value (`comma`) or the arguments
// of an annotation, from after their `(`. Strings (`skip_string`, so a
// triple-quoted one may span lines, and `skip_dollar_slashy_string`) and
// comments are items of their own, and
// line breaks are trivia, as inside parentheses for Groovy's lexer. Returns
// false at the end of input, a string or block comment that does not close, a
// closing bracket that does not match and the end of the budget. It sets
// `*in_string` where it stops inside a dollar-slashy string, whose body no
// caller may read on through as tokens.
//
// A `<` right after a capitalized name, with no bracket open, opens type
// arguments (`new HashMap<String, Integer>()`, `x as Map<K, V>`), whose
// commas end no default value, as in `parentheses_open_statement`. At
// anything type arguments do not hold, the `<` was an operator after all
// (`a = X<b, c = d`); the commas read since cannot be read again, and the
// parameters they started are read as the default's tokens.
static bool skip_lambda_tokens(
    TSLexer *lexer,
    unsigned *items,
    bool comma,
    bool no_close_ahead,
    bool *in_string
) {
    unsigned depth = 0;
    unsigned angles = 0;
    // Whether the last item is a capitalized name with nothing after it yet.
    bool type_name = false;
    // An operand may start here, as in `parentheses_open_statement`: a `/`
    // there opens a slashy string, one item, which the scan function opens
    // where no `=` follows the `/` and the string closes on its line
    // (`slashy_body_follows`), so that its `,` and `)` end nothing
    // (`(a = /x,y/) -> a`). One that does not close on its line ends the
    // look-ahead.
    SlashState state = {.operand = true};
    while (spend_lambda_item(items)) {
        bool slash = false;
        if (is_whitespace(lexer->lookahead) || lexer->lookahead == '\\'
            || lexer->lookahead == '/') {
            type_name = false;
        }
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, &slash)
            || lexer->eof(lexer)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        if (angles > 0
            && (slash || !(is_identifier_char(c) || c == ',' || c == '?'
                           || c == '&' || c == '.' || c == '[' || c == ']'
                           || c == '<' || c == '>'))) {
            angles = 0;
        }
        if (slash) {
            if (state.operand && lexer->lookahead != '=') {
                if (!slashy_body_follows(lexer, true)) {
                    return false;
                }
                advance(lexer);
                slash_state_after_operand(&state);
            } else {
                slash_state_after_operator(&state);
            }
            continue;
        }
        if (c == '\'' || c == '"') {
            if (!skip_string(lexer)) {
                return false;
            }
            slash_state_after_operand(&state);
            continue;
        }
        if (is_identifier_char(c)) {
            type_name = is_capital(c);
            char word[sizeof "synchronized"];
            unsigned len = read_word(lexer, word, sizeof word);
            if (!opens_dollar_slashy(word, len, lexer)) {
                slash_state_after_word(&state, word, len);
            } else if (skip_dollar_slashy_string(lexer, NULL)) {
                slash_state_after_operand(&state);
            } else {
                *in_string = true;
                return false;
            }
            continue;
        }
        bool after_type_name = type_name;
        type_name = false;
        if (c == '(' || c == '[' || c == '{') {
            depth++;
        } else if (c == ')' || c == ']' || c == '}') {
            if (depth == 0) {
                return c == ')';
            }
            depth--;
        } else if (c == '<' && depth == 0 && (after_type_name || angles > 0)) {
            angles++;
        } else if (c == '>' && angles > 0) {
            angles--;
        } else if (c == ',' && depth == 0 && angles == 0 && comma) {
            return true;
        }
        advance(lexer);
        if (slash_state_after_punctuation(&state, c, lexer->lookahead)) {
            advance(lexer);
        }
    }
    return false;
}

// Reads one parameter of a lambda, with the trivia around it, and leaves
// what follows it: annotations, each a name and arguments in parentheses;
// words (`final`, `def`, a type's names, the parameter's name) and the
// punctuation of a type (`.`, `...`, `[]`, type arguments in `<…>`); then a
// default value after `=`. Apart from the name being last, it does not check
// the order: what it takes and `formal_parameter` does not is an `ERROR`
// either way. Returns false where no parameter can be, as at the operator of
// `(a + b)`, the call of `(a.b())` and the literal of `(1)`, and where a
// keyword is in it (`(if)`) other than a wildcard's bound in type arguments
// (`(List<? super T> l)`). Where no name ends it (`(String[])`, `(a.b)`,
// `(int)`, `(final)`), it clears `*named` and goes on: those are a cast's
// types, and the caller reads on past the `)` all the same. A dollar-slashy
// string where a name may be is a literal as well (`($/a/$)`); there, and
// inside one that a default value or an annotation's arguments do not read
// past, it sets `*in_string`.
static bool lambda_parameter_follows(
    TSLexer *lexer,
    unsigned *items,
    bool *named_parameter,
    bool no_close_ahead,
    bool *in_string
) {
    for (;;) {
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
            return false;
        }
        if (lexer->lookahead != '@') {
            break;
        }
        if (!spend_lambda_item(items)) {
            return false;
        }
        advance(lexer);
        // The names of the annotation, joined by dots.
        for (;;) {
            if (!is_identifier_start(lexer->lookahead)) {
                return false;
            }
            if (read_dollar_slashy_open(lexer)) {
                *in_string = true;
                return false;
            }
            if (lexer->lookahead != '.') {
                break;
            }
            advance(lexer);
        }
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
            return false;
        }
        if (lexer->lookahead == '(') {
            advance(lexer);
            if (!skip_lambda_tokens(lexer, items, false, no_close_ahead,
                                    in_string)) {
                return false;
            }
            advance(lexer);
        }
    }
    unsigned angles = 0;
    // Whether the last item is a word that may name the parameter, and how
    // many dots come right before the current item: after one, a word is
    // part of a qualified type (`a.b.C`); after three, a name (`String... a`).
    bool named = false;
    unsigned dots = 0;
    for (;;) {
        int32_t c = lexer->lookahead;
        if (is_identifier_start(c)) {
            char word[sizeof "synchronized"];
            unsigned len = read_word(lexer, word, sizeof word);
            if (opens_dollar_slashy(word, len, lexer)) {
                *in_string = true;
                return false;
            }
            bool modifier = word_is(word, len, "final")
                || word_is(word, len, "def");
            // Inside `<…>` the keywords `extends` and `super` bound a
            // wildcard (`List<? extends Number> l`), as in Apache's
            // `typeArgument`; they name nothing, as `named` below says.
            bool bound = angles > 0
                && (word_is(word, len, "extends")
                    || word_is(word, len, "super"));
            if (!modifier && !bound && is_reserved_lambda_name(word, len)
                && !is_primitive_type(word, len)) {
                return false;
            }
            named = !modifier && angles == 0 && dots != 1
                && !is_reserved_lambda_name(word, len);
            dots = 0;
        } else if (c == '.') {
            named = false;
            dots++;
            advance(lexer);
        } else if (c == '<' || (c == '>' && angles > 0) || c == '[' || c == ']'
                   || (angles > 0 && (c == ',' || c == '?' || c == '&'))) {
            if (c == '<') {
                angles++;
            } else if (c == '>') {
                angles--;
            }
            named = false;
            dots = 0;
            advance(lexer);
        } else {
            break;
        }
        if (!spend_lambda_item(items)
            || !skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
            return false;
        }
    }
    if (angles > 0) {
        return false;
    }
    if (!named) {
        *named_parameter = false;
    }
    if (lexer->lookahead != '=') {
        return true;
    }
    advance(lexer);
    if (lexer->lookahead == '=' || lexer->lookahead == '~') {
        return false;
    }
    return skip_lambda_tokens(lexer, items, true, no_close_ahead, in_string);
}

// SPECIFICATION.md §6.17 — whether the `(` here opens a lambda's parameters:
// what the parentheses hold reads as parameters (`lambda_parameter_follows`)
// and `->` follows the `)`, after Groovy's trivia, line breaks included
// (Apache `NL* ARROW`). The `(` is not consumed when this is called.
//
// A cast's type without a name (`(int)`, `(a.b.C)`) is read through the `)`
// and the trivia after it as well, and turned down only then. The runtime
// counts what the scanner read against the `(`, so an edit in that trivia,
// such as closing a comment that did not close (§6.6), makes it lex the `(`
// again, as it does after `(String)`. `*closed` is set once the look-ahead
// has passed the `)`, so that a caller can read on from where it stopped
// (§6.18), and `*in_string` where it stopped at a dollar-slashy string or
// inside one: no caller reads on from there, and the reading of §6.18 ends
// at the first such string as well (`parentheses_open_statement`).
static bool lambda_parameters_follow(
    TSLexer *lexer,
    bool no_close_ahead,
    bool *closed,
    bool *in_string
) {
    unsigned items = 0;
    bool named = true;
    advance(lexer);
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
        return false;
    }
    if (lexer->lookahead != ')') {
        for (;;) {
            if (!lambda_parameter_follows(lexer, &items, &named,
                                          no_close_ahead, in_string)) {
                return false;
            }
            if (lexer->lookahead == ')') {
                break;
            }
            if (lexer->lookahead != ',') {
                return false;
            }
            advance(lexer);
        }
    }
    advance(lexer);
    *closed = true;
    return arrow_follows(lexer, no_close_ahead) && named;
}

// How many names after its first comma the look-ahead of a multi-assignment
// reads before it leaves the list to the grammar (§6.20). Error recovery may
// re-enter the state after the first name at every name of a broken list,
// and each comma there starts a look-ahead, so the bound keeps such a list
// linear; a method rarely has more parameters.
static const unsigned MULTI_ASSIGNMENT_LOOKAHEAD_NAMES = 16;

// Skips Groovy's trivia as `skip_header_trivia` does, and the form feeds and
// vertical tabs that the grammar's extras skip as well.
static bool skip_multi_assignment_trivia(TSLexer *lexer, bool no_close_ahead) {
    for (;;) {
        if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
            return false;
        }
        if (lexer->lookahead != '\v' && lexer->lookahead != '\f') {
            return true;
        }
        advance(lexer);
    }
}

// SPECIFICATION.md §6.20 — after the first comma of a multi-assignment
// without `def`: whether the multi-assignment may go on. It may not where the
// parentheses go on as a method's parameters do: after a `,` an annotation;
// after a name another name (`(a, int b)`), the `=` of a default value, or
// the `.`, `<` or `[` of a type; after the `)` of a list of names, past
// Groovy's trivia, the `{` or `throws` of a method's body or throws clause
// (`def f(a, b) { }`). Nor may it after a name that anything but a `,` or
// the `)` follows (`(a, b + c)`, `(Ab, Cd>)`), which no list of names holds:
// emitting the token there committed the parser to a multi-assignment inside
// a call's broken arguments (`foo(x, (Ab, Cd>)y)`), and the `ERROR` that
// followed took the rest of the file. Any other text is left to the grammar,
// which ends the reading at the token it did without the look-ahead:
// `(a, b) = l`, a list longer than the bound, `(a, 1)`, `(a, b);`, a `)` at
// the end of the input. What decides is then inside the parentheses or the
// method that the parse keeps, so a reparse that reuses a node of that
// method has read the same text.
static bool multi_assignment_may_go_on(TSLexer *lexer, bool no_close_ahead) {
    for (unsigned names = 0; names < MULTI_ASSIGNMENT_LOOKAHEAD_NAMES; names++) {
        if (!skip_multi_assignment_trivia(lexer, no_close_ahead)) {
            return true;
        }
        if (!is_identifier_start(lexer->lookahead)) {
            return lexer->lookahead != '@';
        }
        // A dollar-slashy string is other text, left to the grammar.
        if (read_dollar_slashy_open(lexer)
            || !skip_multi_assignment_trivia(lexer, no_close_ahead)) {
            return true;
        }
        int32_t c = lexer->lookahead;
        if (c == ')') {
            advance(lexer);
            if (!skip_multi_assignment_trivia(lexer, no_close_ahead)) {
                return true;
            }
            return lexer->lookahead != '{' && !consume_keyword(lexer, "throws");
        }
        if (c == '=') {
            advance(lexer);
            return lexer->lookahead == '=' || lexer->lookahead == '~';
        }
        // Another name, or the `.`, `<` or `[` of a type, makes the
        // parentheses a method's parameters; any other character makes them
        // no list of names.
        if (c != ',') {
            return false;
        }
        advance(lexer);
    }
    return true;
}

// SPECIFICATION.md §6.20 — the first comma of a multi-assignment without
// `def`, after the spaces the scan skipped and any form feeds and vertical
// tabs, which the grammar's extras skip as well. The token is the `,`; what
// is read after it is look-ahead. On `false` the runtime rewinds and the
// in-grammar lexer reads the `,`, which the multi-assignment does not take.
static bool scan_multi_assignment_comma(TSLexer *lexer, bool no_close_ahead) {
    while (lexer->lookahead == '\v' || lexer->lookahead == '\f'
           || is_whitespace(lexer->lookahead)) {
        lexer->advance(lexer, true);
    }
    if (lexer->lookahead != ',') {
        return false;
    }
    advance(lexer);
    lexer->mark_end(lexer);
    if (!multi_assignment_may_go_on(lexer, no_close_ahead)) {
        return false;
    }
    lexer->result_symbol = MULTI_ASSIGNMENT_COMMA;
    return true;
}

// The most strings, one inside the interpolation of the other, that
// `skip_string` follows: `"${a("${b}")}"` holds two. A string nested deeper
// is taken not to close, which ends the look-ahead that reads it. Without
// the bound a run of `"${` that never closes would be read to the end of the
// input from each of its quotes, in time quadratic in its length; real
// strings nest once or twice.
static const unsigned STRING_NESTING_DEPTH = 8;

static bool skip_string_at_depth(TSLexer *lexer, unsigned depth);

// Reads the `${…}` of a double-quoted string, from after its `{` to its
// `}`: braces counted and strings read whole, as Groovy's lexer reads the
// code of an interpolation, which may span lines. A quote in a comment or a
// slashy string there is taken for a string's. Returns false at the end of
// input and at a string that does not close or is nested deeper than
// STRING_NESTING_DEPTH.
static bool skip_interpolation(TSLexer *lexer, unsigned depth) {
    unsigned braces = 0;
    for (;;) {
        if (lexer->eof(lexer)) {
            return false;
        }
        int32_t c = lexer->lookahead;
        if (c == '\'' || c == '"') {
            if (depth == STRING_NESTING_DEPTH
                || !skip_string_at_depth(lexer, depth + 1)) {
                return false;
            }
            continue;
        }
        advance(lexer);
        if (c == '{') {
            braces++;
        } else if (c == '}') {
            if (braces == 0) {
                return true;
            }
            braces--;
        }
    }
}

// Reads a string from its quote: one that closes on its line, where a `\`
// before the line break continues it, as in Groovy, or a triple-quoted one,
// which may span lines. The `${…}` of a double-quoted string is read as
// code (`skip_interpolation`), so a quote, a bracket or a `,` in it is none
// of the string's or of the list around it: `"${xs.join(", ")}"` is one
// string. Returns false at the end of input, or at a line break, before the
// close. `depth` counts the strings this one is nested in.
static bool skip_string_at_depth(TSLexer *lexer, unsigned depth) {
    int32_t quote = lexer->lookahead;
    advance(lexer);
    bool triple = false;
    if (lexer->lookahead == quote) {
        advance(lexer);
        if (lexer->lookahead != quote) {
            return true;
        }
        advance(lexer);
        triple = true;
    }
    unsigned closing = 0;
    for (;;) {
        if (lexer->eof(lexer) || (!triple && is_line_break(lexer->lookahead))) {
            return false;
        }
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '\\') {
            if (!skip_escaped_character(lexer)) {
                return false;
            }
            closing = 0;
        } else if (quote == '"' && c == '$' && lexer->lookahead == '{') {
            advance(lexer);
            if (!skip_interpolation(lexer, depth)) {
                return false;
            }
            closing = 0;
        } else if (c != quote) {
            closing = 0;
        } else if (!triple || ++closing == 3) {
            return true;
        }
    }
}

static bool skip_string(TSLexer *lexer) {
    return skip_string_at_depth(lexer, 0);
}

// SPECIFICATION.md §6.18 — at a `(` that starts a line after an expression,
// where a statement may start: whether the parentheses can open that
// statement, which Groovy reads there as its lexer ends the one before at
// the line break. They cannot, and stay the arguments of a call of that
// expression as they were before, when they are empty, when an element is
// a spread (`*a`) or a named argument (a key of any kind and `:`), when a
// `,` or `else` follows them, or when they hold a `,` with no bracket or
// type arguments open, unless they hold only names and `=` follows them
// (`(a, b) = l`, a multi-assignment) or the first element is a
// parenthesis-free call (`(foo 1, 2)`: a name that is no value keyword,
// any member accesses, and on its line a name other than `in`, `as` and
// `instanceof`, a number or a string). Groovy rejects such a line; Java,
// which much Groovy code is written like, reads a call (`foo` followed by
// `(a, b);`). Before `->` Groovy appends the lambda to the expression
// (`pathElement`'s `nls closureOrLambdaExpression`), so they stay the
// arguments there too. A `<` right after a capitalized name opens type
// arguments (`(data as Map<String, Object>)`, `(Map<K, V>) m`), whose
// commas are no element's; at anything type arguments do not hold, the
// `<` was an operator after all, and the commas count.
//
// The elements are read as Groovy's tokens within the budget of the lambda
// look-ahead. Where the reading cannot go on (the end of the budget or the
// input, a string or a comment that does not close, a `/` that opens an
// element, a dollar-slashy string, a bracket that does not
// match), the parentheses are judged by what was read as if they closed
// there: a statement unless a `,` came, and then a call's arguments unless
// the first element is a parenthesis-free call. That is the reading the
// rest would most often confirm, and it keeps an unfinished line, as while
// typing `(a, b`, read the way it is once closed. The token is the `(`.
static bool parentheses_open_statement(TSLexer *lexer, bool no_close_ahead) {
    advance(lexer);
    lexer->mark_end(lexer);
    unsigned items = 0;
    // Every element so far is one name; a `,` with nothing open came; the
    // first element is a parenthesis-free call.
    bool names = true;
    bool comma = false;
    bool command = false;
    bool known = true;
    for (;;) {
        unsigned depth = 0;
        unsigned angles = 0;
        bool angle_comma = false;
        unsigned questions = 0;
        unsigned tokens = 0;
        bool one_name = false;
        // The first element may be a call's receiver: a name, then member
        // accesses written without spaces. `access` is set while an access
        // waits for its name.
        bool receiver = !comma;
        bool access = false;
        // An operand may start here, where Groovy's lexer reads a `/` as
        // the start of a slashy string: after an operator, an open bracket,
        // `,`, `:` or `?`, not after a name, a number, a string, a closing
        // bracket or `++` and `--`.
        SlashState state = {.operand = true};
        int32_t end = 0;
        while (end == 0) {
            bool slash = false;
            if (!spend_lambda_item(&items)
                || !skip_header_trivia(lexer, true, no_close_ahead, NULL,
                                       tokens > 0 ? &slash : NULL)
                || lexer->eof(lexer)) {
                known = false;
                break;
            }
            int32_t c = lexer->lookahead;
            // The slashy string is one token, opened as the scan function
            // opens one: no `=` after the `/` and a closing `/` on its line
            // (`slashy_body_follows`). Its `,` and `:` are none of the
            // element's (`(text =~ /a, b/)`); one that does not close on its
            // line ends the reading.
            if (slash && state.operand && angles == 0 && c != '=') {
                if (!slashy_body_follows(lexer, true)) {
                    known = false;
                    break;
                }
                advance(lexer);
                tokens++;
                one_name = false;
                access = false;
                receiver = false;
                slash_state_after_operand(&state);
                continue;
            }
            if (angles > 0
                && (slash || !(is_identifier_start(c) || c == '.' || c == ','
                               || c == '?' || c == '&' || c == '['
                               || c == ']' || c == '<' || c == '>'))) {
                angles = 0;
                if (angle_comma) {
                    comma = true;
                    names = false;
                    if (!command) {
                        return false;
                    }
                }
            }
            if ((c == ')' || c == ',') && depth == 0 && angles == 0) {
                end = c;
                if (c == ',') {
                    advance(lexer);
                }
                break;
            }
            bool first = tokens == 0;
            tokens++;
            if (!first) {
                one_name = false;
            }
            bool was_access = access;
            access = false;
            if (slash) {
                receiver = false;
                slash_state_after_operator(&state);
                continue;
            }
            if (c == ',') {
                advance(lexer);
                slash_state_after_operator(&state);
                if (angles > 0) {
                    angle_comma = true;
                }
                continue;
            }
            if (c == '(' || c == '[' || c == '{') {
                advance(lexer);
                slash_state_after_operator(&state);
                receiver = false;
                depth++;
                continue;
            }
            if (c == ')' || c == ']' || c == '}') {
                if (depth == 0) {
                    known = false;
                    break;
                }
                advance(lexer);
                slash_state_after_operand(&state);
                depth--;
                continue;
            }
            if (c == '\'' || c == '"') {
                if (!skip_string(lexer)) {
                    known = false;
                    break;
                }
                slash_state_after_operand(&state);
                receiver = false;
                continue;
            }
            if (c == '*' && first) {
                return false;
            }
            if (c == '.' || c == '?' || c == '*') {
                advance(lexer);
                // The characters after the first are read below; the state
                // after them is that of a member access where `dot` is set.
                slash_state_after_operator(&state);
                bool dot = c == '.';
                if (c == '.') {
                    if (lexer->lookahead == '&' || lexer->lookahead == '@') {
                        advance(lexer);
                    }
                } else if (c == '?') {
                    if (lexer->lookahead == '?') {
                        advance(lexer);
                    }
                    dot = lexer->lookahead == '.';
                    if (dot) {
                        advance(lexer);
                    } else if (lexer->lookahead == ':') {
                        advance(lexer);
                    } else if (lexer->lookahead == '[') {
                        advance(lexer);
                        depth++;
                    } else if (depth == 0 && angles == 0) {
                        questions++;
                    }
                } else {
                    dot = lexer->lookahead == '.';
                    if (dot) {
                        advance(lexer);
                    }
                }
                if (dot && lexer->lookahead == '@') {
                    advance(lexer);
                }
                if (dot) {
                    state.operand = false;
                    state.member = true;
                }
                access = dot && receiver && was_access
                    && is_identifier_start(lexer->lookahead);
                receiver = access;
                continue;
            }
            if (c == ':') {
                advance(lexer);
                slash_state_after_operator(&state);
                receiver = false;
                if (lexer->lookahead == ':') {
                    advance(lexer);
                    state.member = true;
                } else if (depth == 0 && angles == 0 && !command) {
                    if (questions == 0) {
                        return false;
                    }
                    questions--;
                }
                continue;
            }
            if (c == '>') {
                advance(lexer);
                slash_state_after_operator(&state);
                receiver = false;
                if (angles > 0 && --angles == 0) {
                    angle_comma = false;
                }
                continue;
            }
            if (!is_identifier_char(c)) {
                advance(lexer);
                if (slash_state_after_punctuation(&state, c,
                                                  lexer->lookahead)) {
                    advance(lexer);
                }
                receiver = false;
                continue;
            }
            char word[sizeof "synchronized"];
            unsigned len = read_word(lexer, word, sizeof word);
            if (opens_dollar_slashy(word, len, lexer)) {
                known = false;
                break;
            }
            slash_state_after_word(&state, word, len);
            bool name = is_identifier_start(c);
            if (first) {
                one_name = name;
            }
            if (lexer->lookahead == '<' && (angles > 0 || is_capital(c))) {
                advance(lexer);
                slash_state_after_operator(&state);
                angles++;
                receiver = false;
                continue;
            }
            receiver = receiver && name && (first || was_access)
                && !(first && is_value_keyword(word, len, lexer->lookahead));
            if (!receiver) {
                continue;
            }
            if (is_member_access_start(lexer->lookahead)) {
                access = true;
                continue;
            }
            receiver = false;
            if (!is_inline_space(lexer->lookahead)
                && lexer->lookahead != '\\') {
                continue;
            }
            if (!skip_same_line_space(lexer, false)) {
                known = false;
                break;
            }
            int32_t next = lexer->lookahead;
            if (is_identifier_start(next)) {
                char argument[sizeof "synchronized"];
                unsigned argument_len =
                    read_word(lexer, argument, sizeof argument);
                slash_state_after_word(&state, argument, argument_len);
                tokens++;
                one_name = false;
                command = !word_is(argument, argument_len, "in")
                    && !word_is(argument, argument_len, "as")
                    && !word_is(argument, argument_len, "instanceof");
                // The argument is a dollar-slashy string (`(foo $/a/$, 2)`),
                // where the reading cannot go on.
                if (opens_dollar_slashy(argument, argument_len, lexer)) {
                    known = false;
                    break;
                }
            } else if (is_literal_start(next)) {
                command = true;
            }
        }
        if (!known) {
            if (angles > 0 && angle_comma) {
                comma = true;
                names = false;
            }
            return !comma || command;
        }
        if (!one_name) {
            names = false;
        }
        if (end == ')') {
            if (tokens == 0 && !comma) {
                return false;
            }
            break;
        }
        comma = true;
        if (!names && !command) {
            return false;
        }
    }
    if (comma && !names && !command) {
        return false;
    }
    advance(lexer);
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
        return !comma || command;
    }
    int32_t next = lexer->lookahead;
    if (next == ',') {
        return false;
    }
    if (next == '-') {
        advance(lexer);
        if (lexer->lookahead == '>') {
            return false;
        }
    } else if (is_identifier_start(next)) {
        char word[sizeof "else"];
        if (word_is(word, read_word(lexer, word, sizeof word), "else")) {
            return false;
        }
    }
    if (!comma || command) {
        return true;
    }
    if (next != '=') {
        return false;
    }
    advance(lexer);
    return lexer->lookahead != '=' && lexer->lookahead != '~';
}

// SPECIFICATION.md §6.18 — where STATEMENT_PAREN is valid but no expression
// ends, as at a statement's start, the `(` after a line break is read as far
// as `parentheses_open_statement` reads it where it decides: through the
// `)` that closes it (from where the lambda look-ahead stopped, `closed`
// once that passed it), the trivia after it, and the word or the
// character after a `=` or `-` there. The runtime counts what the scanner
// read against the in-grammar `(` that follows, so a reparse that reused
// the statement before reads that `(` again after an edit that would
// change the decision, as it does where the decision is made.
static void read_parentheses_extent(
    TSLexer *lexer,
    bool closed,
    bool no_close_ahead
) {
    unsigned items = 0;
    unsigned depth = 0;
    while (!closed) {
        bool slash = false;
        if (!spend_lambda_item(&items)
            || !skip_header_trivia(lexer, true, no_close_ahead, NULL, &slash)
            || lexer->eof(lexer)) {
            return;
        }
        int32_t c = lexer->lookahead;
        if (slash) {
            continue;
        }
        if (c == '\'' || c == '"') {
            if (!skip_string(lexer)) {
                return;
            }
            continue;
        }
        if (is_identifier_char(c)) {
            // `parentheses_open_statement` reads no further than the `$/` of
            // a dollar-slashy string.
            if (read_dollar_slashy_open(lexer)) {
                return;
            }
            continue;
        }
        advance(lexer);
        if (c == '(' || c == '[' || c == '{') {
            depth++;
        } else if (c == ')' || c == ']' || c == '}') {
            if (depth == 0) {
                if (c != ')') {
                    return;
                }
                closed = true;
            } else {
                depth--;
            }
        }
    }
    if (!skip_header_trivia(lexer, true, no_close_ahead, NULL, NULL)) {
        return;
    }
    if (is_identifier_start(lexer->lookahead)) {
        read_word(lexer, NULL, 0);
    } else if (lexer->lookahead == '=' || lexer->lookahead == '-') {
        advance(lexer);
    }
}

// SPECIFICATION.md §6.2 — the most escaped slashes (`\/`) the look-ahead
// for a slashy string's closing `/` reads past; at the last of them it takes
// the string to close. The look-ahead runs from every `/` where a slashy
// string may start, which during error recovery is every `/`, and an escaped
// slash ends none of them: a line of `\/` was read from each of its slashes
// to its end, in time quadratic in its length. With the bound a character is
// read from at most this many slashes before it. Regexes in real code hold a
// few (at most five on a line of the Java set, none in the Groovy one).
static const unsigned SLASHY_LOOKAHEAD_ESCAPED_SLASHES = 64;

// After a `/` that opens no comment: guards (1) to (3) of the slashy
// opener in `tree_sitter_groovy_dekobon_external_scanner_scan`, a body that is
// neither empty nor starts with whitespace, and a closing `/` on the line
// (looked for without keeping what is read), or
// SLASHY_LOOKAHEAD_ESCAPED_SLASHES escaped slashes before the end of it.
static bool slashy_body_follows(TSLexer *lexer, bool lines) {
    if (lexer->lookahead == '/'
        || (!lines && is_whitespace(lexer->lookahead))) {
        return false;
    }
    unsigned escaped_slashes = 0;
    while (!lexer->eof(lexer)) {
        if (lexer->lookahead == '\\') {
            advance(lexer);
            if (lexer->lookahead == '/'
                && ++escaped_slashes == SLASHY_LOOKAHEAD_ESCAPED_SLASHES) {
                return true;
            }
            skip_escaped_character(lexer);
            continue;
        }
        if (lexer->lookahead == '/') {
            return true;
        }
        if (!lines && is_line_break(lexer->lookahead)) {
            return false;
        }
        advance(lexer);
    }
    return false;
}

// SPECIFICATION.md §6.19 — the most comments the scan after a lambda body's
// `}` reads before the token it decides on; one more ends the body. Where
// the body goes on past them, the scan runs again after each comment of the
// run, and the comment's token reads the rest of the run as well
// (`read_deferred_extent`), so a run of k comments costs k * k + k comment
// reads: 72 here, where 256 allowed 65,792. Runs of comments between a
// lambda's body and an operator that continues it are a line or two in
// practice; a longer run ends the body as a statement after it would.
static const unsigned LAMBDA_BODY_END_COMMENTS = 8;

// SPECIFICATION.md §6.19 — how a scan after a lambda body's `}` ends, and
// after the `}` of a block that opens a statement (§6.22).
// BODY_END_DECIDE is the scan in a state after the `}`, where the braces may
// still be a closure that an expression continues. BODY_END_HAND_BACK is the
// scan in a state where only LAMBDA_BODY_END, or STATEMENT_BLOCK_END, is
// valid: after the block of a lambda without parentheses, or of a statement,
// that the parser reused from an old tree, or that a stack version read
// beside one reading the braces as a closure or as another block. It
// ends the body where the scan in every state after the `}` would, and
// elsewhere emits LAMBDA_BODY_REREAD, which no rule takes. A reused block is
// then broken down and the `}` lexed again in the state after it, and the
// version reading a block ends, as it would on the token after the `}`.
// BODY_END_READ only reads, as far as a scan in any state would, and marks
// and skips nothing.
typedef enum {
    BODY_END_DECIDE,
    BODY_END_HAND_BACK,
    BODY_END_READ,
} BodyEndScan;

// SPECIFICATION.md §6.19 — the end of the scan after a lambda body's `}`
// when an expression continues there: false, so that the in-grammar lexer
// reads the operator, unless the scan read past the start of the next
// token (`deferred`: a comment came first, or the operator is a scanner
// token whose end it passed). Then the empty UNTERMINATED_COMMENT sets the
// flag that has the next call read that token without deciding again.
// Without the scanner's memory it ends the body instead.
static bool lambda_body_continues(
    Scanner *scanner,
    TSLexer *lexer,
    BodyEndScan mode,
    bool deferred
) {
    if (mode == BODY_END_HAND_BACK) {
        lexer->result_symbol = LAMBDA_BODY_REREAD;
        return true;
    }
    if (!deferred) {
        return false;
    }
    if (scanner == NULL || mode == BODY_END_READ) {
        lexer->result_symbol = LAMBDA_BODY_END;
        return true;
    }
    scanner->lambda_body_continues = true;
    lexer->result_symbol = UNTERMINATED_COMMENT;
    return true;
}

// The end of the scan after a lambda body's `}` when `!in` or the `[` of a
// subscript continues it, scanner tokens the scan has just read: that
// token, or the deferral of `lambda_body_continues` when a comment came
// first.
static bool lambda_body_continues_with(
    Scanner *scanner,
    TSLexer *lexer,
    BodyEndScan mode,
    bool comment,
    TSSymbol token
) {
    if (comment || mode != BODY_END_DECIDE) {
        return lambda_body_continues(scanner, lexer, mode, true);
    }
    lexer->mark_end(lexer);
    lexer->result_symbol = token;
    return true;
}

// Whether `token` counts as valid for the scan after a lambda body's `}`.
// A scan that only reads takes every token as valid, so that it reads as far
// as a scan in any state. The scan in the state after a shifted block takes
// as invalid the tokens that end the body where they are valid
// (STATEMENT_PAREN, RECORD_KEYWORD) and as valid the others, which let it go
// on, so that it ends the body only where the scan in every state after the
// `}` would.
static inline bool token_valid(
    const bool *valid_symbols,
    BodyEndScan mode,
    enum TokenType token
) {
    switch (mode) {
        case BODY_END_READ:
            return true;
        case BODY_END_HAND_BACK:
            return token != STATEMENT_PAREN && token != RECORD_KEYWORD;
        default:
            return valid_symbols[token];
    }
}

// Whether type parameters, which open a generic method or constructor,
// start at this `<`: an annotation or a name followed by `>`, `,` or
// `extends`, on the line. Consumes what it reads.
static bool type_parameters_follow(TSLexer *lexer) {
    advance(lexer);
    if (!skip_same_line_space(lexer, false)) {
        return false;
    }
    if (lexer->lookahead == '@') {
        return true;
    }
    if (!is_identifier_start(lexer->lookahead)) {
        return false;
    }
    char word[sizeof "extends"];
    read_word(lexer, word, sizeof word);
    if (!skip_same_line_space(lexer, false)) {
        return false;
    }
    if (lexer->lookahead == '>' || lexer->lookahead == ',') {
        return true;
    }
    unsigned len = read_word(lexer, word, sizeof word);
    return word_is(word, len, "extends");
}

// SPECIFICATION.md §6.19 — LAMBDA_BODY_END, empty, after the `}` of a
// lambda's brace body unless an expression continues there. The trivia
// after the `}` is read, comments included, and then one operator or
// word. On the line of the `}` any operator, `(`, `[`, `{`, `instanceof`
// and `as` or `in` that open a type or an expression (§6.16) continue
// it. After a line break where Groovy's lexer keeps it, so do the
// operators that Groovy's grammar takes after `nls`, but what may start
// what follows: where a statement may start (STATEMENT_PAREN is valid,
// §6.18) `(`, `+`, `-`, `++`, `--` and a slashy string, and where a
// statement or a class member may (STATEMENT_PAREN or RECORD_KEYWORD is
// valid, §6.7) `{` and the type parameters of a generic method. A `[` on a
// later line ends it everywhere (§6.9). Neither token is valid inside
// parentheses and brackets, where Groovy's lexer drops the line break,
// and interpolations. `)`, `]`, `}`, `,`,
// `;`, `:` other than `::`, any other word, a literal, `!` other than
// `!=`, `!in` and `!instanceof`, the end of the input, a `/*` that does
// not close and more than LAMBDA_BODY_END_COMMENTS comments end it. The
// token is empty and ends at the `}`, so that the lambda keeps its range
// and a comment after the body it ends stays outside. A call reads the
// trivia, at most LAMBDA_BODY_END_COMMENTS comments of it, and one word,
// a few characters or the start of type parameters. `mode` says how it
// ends (BodyEndScan). `end` is the token it emits where the braces end:
// LAMBDA_BODY_END, or STATEMENT_BLOCK_END after a block that opens a
// statement (§6.22), which is decided the same way.
static bool scan_lambda_body_end(
    Scanner *scanner,
    TSLexer *lexer,
    const bool *valid_symbols,
    bool no_close_ahead,
    BodyEndScan mode,
    TSSymbol end
) {
    // A scan that only reads skips nothing either: the token it follows
    // must keep its start.
    bool decides = mode != BODY_END_READ;
    if (decides) {
        lexer->mark_end(lexer);
    }
    lexer->result_symbol = end;
    bool crossed_line = false;
    bool comment = false;
    unsigned comments = 0;
    for (;;) {
        if (!skip_same_line_space(lexer, decides)) {
            return true;
        }
        if (is_line_break(lexer->lookahead)) {
            crossed_line = true;
            lexer->advance(lexer, decides);
            continue;
        }
        if (lexer->lookahead != '/') {
            break;
        }
        if (comments == LAMBDA_BODY_END_COMMENTS) {
            return true;
        }
        advance(lexer);
        if (lexer->lookahead == '/') {
            while (!lexer->eof(lexer) && !is_line_break(lexer->lookahead)) {
                advance(lexer);
            }
        } else if (lexer->lookahead == '*') {
            if (no_close_ahead || !skip_comment_body(lexer)) {
                return true;
            }
        } else if (lexer->lookahead == '=' || !crossed_line
                   || !token_valid(valid_symbols, mode, STATEMENT_PAREN)
                   || !slashy_body_follows(lexer, false)) {
            // On the line of the `}` a `/` divides, as after any expression
            // (`x -> { a } /2/ 3`): Groovy's lexer starts no regex after a
            // `}`. A slashy string starts only the statement on a later line.
            return lambda_body_continues(scanner, lexer, mode, comment);
        } else {
            return true;
        }
        comment = true;
        comments++;
    }
    bool statement_follows = crossed_line
        && token_valid(valid_symbols, mode, STATEMENT_PAREN);
    bool member_follows = statement_follows
        || (crossed_line
            && token_valid(valid_symbols, mode, RECORD_KEYWORD));
    int32_t c = lexer->lookahead;
    if (lexer->eof(lexer)) {
        return true;
    }
    if (is_identifier_start(c)) {
        char word[sizeof "instanceof"];
        unsigned len = read_word(lexer, word, sizeof word);
        if (word_is(word, len, "instanceof")) {
            return lambda_body_continues(scanner, lexer, mode, comment);
        }
        // What `scan_contextual_word` decides for these words, and reads.
        bool as = word_is(word, len, "as");
        if ((as || word_is(word, len, "in"))
            && token_valid(valid_symbols, mode, as ? AS_KEYWORD : IN_KEYWORD)
            && (as ? skip_header_trivia(lexer, true, no_close_ahead, NULL,
                                        NULL)
                         && is_identifier_start(lexer->lookahead)
                   : expression_follows(lexer, false, no_close_ahead))) {
            return lambda_body_continues(scanner, lexer, mode, true);
        }
        lexer->result_symbol = end;
        return true;
    }
    switch (c) {
        case '<':
            return (member_follows && type_parameters_follow(lexer))
                || lambda_body_continues(scanner, lexer, mode, comment);
        case '.':
        case '?':
        case '*':
        case '%':
        case '>':
        case '=':
        case '&':
        case '|':
        case '^':
            return lambda_body_continues(scanner, lexer, mode, comment);
        case '(':
            return statement_follows
                || lambda_body_continues(scanner, lexer, mode, comment);
        case '{':
            return member_follows
                || lambda_body_continues(scanner, lexer, mode, comment);
        case '+':
        case '-':
            advance(lexer);
            if (statement_follows && lexer->lookahead != '=') {
                return true;
            }
            return lambda_body_continues(scanner, lexer, mode, comment);
        case ':':
            advance(lexer);
            return lexer->lookahead != ':'
                || lambda_body_continues(scanner, lexer, mode, comment);
        case '[':
            // A `[` on a later line opens no subscript (§6.9).
            advance(lexer);
            if (crossed_line
                || !token_valid(valid_symbols, mode, SUBSCRIPT_OPEN)) {
                return true;
            }
            return lambda_body_continues_with(scanner, lexer, mode, comment,
                                              SUBSCRIPT_OPEN);
        case '!':
            advance(lexer);
            if (lexer->lookahead == '=') {
                return lambda_body_continues(scanner, lexer, mode, comment);
            }
            if (is_identifier_start(lexer->lookahead)) {
                char word[sizeof "instanceof"];
                unsigned len = read_word(lexer, word, sizeof word);
                int32_t next = lexer->lookahead;
                if (word_is(word, len, "instanceof") && is_whitespace(next)) {
                    return lambda_body_continues(scanner, lexer, mode,
                                                 comment);
                }
                if (word_is(word, len, "in")
                    && token_valid(valid_symbols, mode, NOT_IN)
                    && (is_whitespace(next) || next == '[' || next == '('
                        || next == '{')) {
                    return lambda_body_continues_with(scanner, lexer, mode,
                                                      comment, NOT_IN);
                }
            }
            return true;
        default:
            return true;
    }
}

// SPECIFICATION.md §6.19 — at a `}` that closes a list of statements, as a
// lambda's brace body does (STATEMENT_PAREN is valid before it), and that a
// comment or a line escape follows on its line: reads past it as far as
// `scan_lambda_body_end` would read after it, with every token valid so
// that no state reads farther. The scan then returns false and the
// in-grammar lexer reads the `}`, whose look-ahead the runtime counts from
// what was read here, so that a node that ends at the `}` is reused only
// while the text the body's end depends on is unchanged, in the states
// where LAMBDA_BODY_END is valid after the `}` and in all others. A reused
// closure that the parse had continued as an expression would otherwise
// take the comment as an extra in that state, where LAMBDA_BODY_END is not
// valid, before the parser breaks it down again and decides the body's end
// after the comment. With no comment there the parser breaks the closure
// down before it shifts anything, so only the spaces are read. A comment on
// a later line is not read past, which would cost the read at every `}`
// that ends a line: that leaves the one case where a reparse differs from
// a parse from scratch (§6.19). The `}` of an interpolation or a class
// body, which no lambda's body ends with, is not read past either: after
// an interpolation's `}` a string goes on, where a `/*` opens no comment
// that could set the §6.6 bit, and every such `}` would read to the end of
// the input. After a `}` of statements the comment is lexed next, so an
// unterminated one is read to the end once and sets the bit.
static void read_lambda_body_end_extent(TSLexer *lexer, bool no_close_ahead) {
    advance(lexer);
    while (is_inline_space(lexer->lookahead)) {
        advance(lexer);
    }
    if (lexer->lookahead == '/' || lexer->lookahead == '\\') {
        scan_lambda_body_end(NULL, lexer, NULL, no_close_ahead,
                             BODY_END_READ, LAMBDA_BODY_END);
    }
}

// SPECIFICATION.md §6.19 — after a comment that a scan deferred to
// (`body_continues`), which ended the token at the comment's end: reads on
// as far as the decision after the `}` read, so that the comment, which a
// reparse may reuse in the state that decides, is lexed again when the
// text after it changes.
// SPECIFICATION.md §6.23 — past JUMP_STATEMENT_END, whose end the caller
// has marked: reads on over line breaks and comments to the first
// character of the next token, so that the runtime counts that text
// against the statement too and an edit there parses it again. A reparse
// that reused the ended statement in one of two stack versions (after
// `(a) { }`, a cast or a call) let that version reach an error in the next
// line before the other, and kept the other reading, where a parse from
// scratch keeps the first.
static void read_jump_statement_end_extent(
    TSLexer *lexer,
    bool no_close_ahead
) {
    bool slash = false;
    skip_header_trivia(lexer, true, no_close_ahead, NULL, &slash);
}

static void read_deferred_extent(
    TSLexer *lexer,
    bool body_continues,
    bool no_close_ahead
) {
    if (body_continues) {
        TSSymbol comment = lexer->result_symbol;
        scan_lambda_body_end(NULL, lexer, NULL, no_close_ahead,
                             BODY_END_READ, LAMBDA_BODY_END);
        lexer->result_symbol = comment;
    }
}

void *tree_sitter_groovy_dekobon_external_scanner_create(void) {
    return ts_calloc(1, sizeof(Scanner));
}

void tree_sitter_groovy_dekobon_external_scanner_destroy(void *payload) {
    ts_free(payload);
}

// Length of the serialized state while every flag is unset. It must
// stay 0: the runtime passes that length before the first external
// token.
static const unsigned UNSET_STATE_LENGTH = 0;

// Length of the serialized state once a flag is set: one byte.
static const unsigned SET_STATE_LENGTH = 1;

// The bits of that byte: `no_comment_close_ahead`,
// `lambda_body_continues`, `jump_continues` and
// `slashy_after_comment_newline`. Any other bit is read as unset.
static const char NO_COMMENT_CLOSE_AHEAD_BIT = 1;
static const char LAMBDA_BODY_CONTINUES_BIT = 2;
static const char JUMP_CONTINUES_BIT = 4;
static const char SLASHY_AFTER_COMMENT_NEWLINE_BIT = 8;

unsigned tree_sitter_groovy_dekobon_external_scanner_serialize(
    void *payload,
    char *buffer
) {
    const Scanner *scanner = payload;
    if (scanner == NULL
        || (!scanner->no_comment_close_ahead
            && !scanner->lambda_body_continues
            && !scanner->jump_continues
            && !scanner->slashy_after_comment_newline)) {
        return UNSET_STATE_LENGTH;
    }
    buffer[0] = (char)((scanner->no_comment_close_ahead
                        ? NO_COMMENT_CLOSE_AHEAD_BIT : 0)
                        | (scanner->lambda_body_continues
                           ? LAMBDA_BODY_CONTINUES_BIT : 0)
                        | (scanner->jump_continues ? JUMP_CONTINUES_BIT : 0)
                        | (scanner->slashy_after_comment_newline
                           ? SLASHY_AFTER_COMMENT_NEWLINE_BIT : 0));
    return SET_STATE_LENGTH;
}

void tree_sitter_groovy_dekobon_external_scanner_deserialize(
    void *payload,
    const char *buffer,
    unsigned length
) {
    Scanner *scanner = payload;
    if (scanner != NULL) {
        bool set = length == SET_STATE_LENGTH;
        scanner->no_comment_close_ahead =
            set && (buffer[0] & NO_COMMENT_CLOSE_AHEAD_BIT) != 0;
        scanner->lambda_body_continues =
            set && (buffer[0] & LAMBDA_BODY_CONTINUES_BIT) != 0;
        scanner->jump_continues =
            set && (buffer[0] & JUMP_CONTINUES_BIT) != 0;
        scanner->slashy_after_comment_newline =
            set && (buffer[0] & SLASHY_AFTER_COMMENT_NEWLINE_BIT) != 0;
    }
}

// The decisions, in the order this function tries them; the first that
// applies returns:
//   1. Inside a string body (STRING_BODY valid), nothing (§6.10).
//   2. After the `}` of a lambda's brace body or of a statement's block,
//      LAMBDA_BODY_END or STATEMENT_BLOCK_END (`scan_lambda_body_end`,
//      §6.19, §6.22), unless the previous call set `lambda_body_continues`,
//      which this one clears before reading on as after an expression.
//   3. At `<` after a cast type's name, CAST_TYPE_ARGUMENTS_OPEN, and after
//      another generic type's, TYPE_ARGUMENTS_OPEN (§3.2.2).
//   4. At a member access after a statement's first name,
//      COMMAND_PATH_START (§6.14).
//   4a. After `return`, `break` or `continue`, past spaces, tabs and line
//      escapes, at a line break or the end of input, JUMP_STATEMENT_END
//      (§6.23); else on, with the spaces skipped.
//   5. Past spaces, tabs and line escapes only: SUBSCRIPT_OPEN at `[`
//      (§6.9); a `(` on the line where STATEMENT_PAREN is valid turned down
//      (§6.18); at a name, COMMAND_NAMED_KEY or CONTEXTUAL_KEY (§6.8,
//      §6.16).
//   6. Past whitespace and line breaks, at a name: a statement's first name
//      (`scan_statement_start`: COMMAND_STATEMENT_START, DECLARATION_START
//      and the keywords and array type value that name may be); else
//      `record`, `trait`, `sealed` or `short[]` (§6.7, §6.13, §6.15); else
//      `var`, `yield`, `in`, `as` or `int[]` where no value and no bare
//      lambda may start (§6.16); else COMMAND_VALUE_START,
//      BARE_LAMBDA_START or ARRAY_VALUE_START (`scan_value_start`, which
//      decides the §6.16 words there).
//   7. `!in`, NOT_IN (§6.12); `)`, CAST_CLOSE or RELATIONAL_CLOSE (§6.11).
//   8. `(`: STATEMENT_PAREN where an expression ends (§6.18), LAMBDA_START
//      where none does (§6.17), which also reads the extent of §6.18.
//   9. `}` where STATEMENT_PAREN is valid: the extent of §6.19, no token.
//  10. Any other character but `/`: MULTI_ASSIGNMENT_COMMA (§6.20).
//  11. `/`: a line, block or Groovydoc comment, UNTERMINATED_COMMENT, `/=`
//      turned down, or SLASHY_STRING_START (§6.6, §6.2).
bool tree_sitter_groovy_dekobon_external_scanner_scan(
    void *payload,
    TSLexer *lexer,
    const bool *valid_symbols
) {
    Scanner *scanner = payload;
    bool body_continues = false;

    // SPECIFICATION.md §6.23 — set by the marker emitted at the end of
    // `return`, `break` or `continue` where a block comment and spaces
    // follow and then something else on the line. This call, at the same
    // position, reads the comment; whatever it emits clears the flag.
    bool jump_continues = scanner != NULL && scanner->jump_continues;
    if (jump_continues) {
        scanner->jump_continues = false;
    }

    // SPECIFICATION.md §6.10 — STRING_BODY is valid only between the
    // quotes of a string whose body is lexed in pieces. Comments are
    // extras, so the parser offers them there as well, and this
    // function is called at every piece boundary. Without this check a
    // body that starts with `//` or `/*`, or has one after an
    // interpolation, lost that text to a comment token. The body
    // belongs to the in-grammar lexer.
    if (valid_symbols[STRING_BODY] && !valid_symbols[ERROR_SENTINEL]) {
        return false;
    }

    // SPECIFICATION.md §6.19 — after the `}` of a lambda's brace body,
    // the one place LAMBDA_BODY_END is valid, and §6.22 after the `}` of a
    // block that opens a statement, the one place STATEMENT_BLOCK_END is;
    // no state takes both. Every token that may follow there is decided by
    // `scan_lambda_body_end` or read by the in-grammar lexer, or, once the
    // UNTERMINATED_COMMENT it emitted has set `lambda_body_continues`, by
    // the branches below, which read it as they would after any expression;
    // those that might take the word `as` or `in` for something else are
    // skipped. The flag is set only by a scan here and read by the next one,
    // at the same position and parse state.
    //
    // Where the braces may still be a closure that an expression continues,
    // NOT_IN is valid. Where it is not, only the token is valid, and
    // the scan may not defer to a comment (BODY_END_HAND_BACK): a comment
    // shifted after a reused block would stay before the token once the
    // parser broke the block down, and the token be decided after it.
    if ((valid_symbols[LAMBDA_BODY_END] || valid_symbols[STATEMENT_BLOCK_END])
        && !valid_symbols[ERROR_SENTINEL]) {
        TSSymbol end = valid_symbols[STATEMENT_BLOCK_END]
            ? STATEMENT_BLOCK_END : LAMBDA_BODY_END;
        if (!valid_symbols[NOT_IN]) {
            return scan_lambda_body_end(scanner, lexer, valid_symbols,
                                        no_comment_close_ahead(scanner),
                                        BODY_END_HAND_BACK, end);
        }
        if (scanner == NULL || !scanner->lambda_body_continues) {
            return scan_lambda_body_end(scanner, lexer, valid_symbols,
                                        no_comment_close_ahead(scanner),
                                        BODY_END_DECIDE, end);
        }
        scanner->lambda_body_continues = false;
        body_continues = true;
    }

    // SPECIFICATION.md §3.2.2 — the `<` right after the name of a cast's
    // type, where type arguments that close follow it, and of any other
    // generic type, where they close or run past the read. On false the
    // runtime rewinds and the in-grammar lexer reads the `<`: a
    // comparison's where one may follow the name (`(i<n)`, whose `i` may
    // also be a cast's type), as the grammar ranks it above the `<` of type
    // arguments, and else the type arguments'. Past the read a cast's type
    // compares (`f(x as B<c, …`), and another type keeps its arguments, as
    // it did before a comparison could follow its name. After whitespace
    // the `<` is never one of type arguments.
    //
    // The same read runs where the token is not valid but an expression has
    // ended (NOT_IN is valid): a reparse that reuses the cast, or its type,
    // lexes the `<` after it in that state, where no scan decided it. Reading
    // as far there makes the runtime count the type arguments against the
    // in-grammar `<` in both states, so an edit that closes them
    // (`y = foo as List<String`, then `>`) breaks the cast down and lexes the
    // `<` again where the token is valid. No other scanner token starts with
    // `<`.
    if ((valid_symbols[CAST_TYPE_ARGUMENTS_OPEN]
         || valid_symbols[TYPE_ARGUMENTS_OPEN] || valid_symbols[NOT_IN])
        && !valid_symbols[ERROR_SENTINEL] && lexer->lookahead == '<') {
        advance(lexer);
        lexer->mark_end(lexer);
        // `<>` is a diamond, whose `<` the in-grammar lexer reads.
        if (lexer->lookahead == '>') {
            return false;
        }
        TypeArguments arguments =
            read_type_arguments(lexer, no_comment_close_ahead(scanner));
        if (arguments == ARGUMENTS_CLOSE
            && valid_symbols[CAST_TYPE_ARGUMENTS_OPEN]) {
            lexer->result_symbol = CAST_TYPE_ARGUMENTS_OPEN;
            return true;
        }
        if (arguments != ARGUMENTS_COMPARE
            && valid_symbols[TYPE_ARGUMENTS_OPEN]) {
            lexer->result_symbol = TYPE_ARGUMENTS_OPEN;
            return true;
        }
        return false;
    }

    // SPECIFICATION.md §6.14 — a member access directly after the first
    // name of a statement. No other token starts with one of its
    // characters.
    if (valid_symbols[COMMAND_PATH_START] && !valid_symbols[ERROR_SENTINEL]
        && is_member_access_start(lexer->lookahead)) {
        return scan_command_path_start(lexer, no_comment_close_ahead(scanner));
    }

    // SPECIFICATION.md §6.23 — after `return`, `break` or `continue`, the
    // only place JUMP_STATEMENT_END is valid, a line break or the end of
    // input past spaces, tabs and line escapes ends the statement, as
    // Groovy's parser takes the value or the label only on the keyword's
    // line. The token is empty and the statement's own, so the runtime
    // counts the line break against the statement. Anything else falls
    // through with the spaces skipped: a line comment, and a block comment
    // that the line ends right after, which Groovy's lexer reads as the line
    // break, end the statement before them in the comment branch below.
    // Past the token the scan reads on to the next token
    // (`read_jump_statement_end_extent`).
    //
    // The token ends at the keyword, before the spaces, so that the
    // statement ends where it does without the token (`continue;`): a
    // reparse that reuses a statement lexed without it, where the edit
    // follows the spaces, gives the same range as a parse from scratch.
    // Every token that a branch below emits after the spaces marks its own
    // end.
    if (valid_symbols[JUMP_STATEMENT_END] && !valid_symbols[ERROR_SENTINEL]) {
        lexer->mark_end(lexer);
        if (!skip_same_line_space(lexer, true)) {
            return false;
        }
        if (lexer->eof(lexer) || is_line_break(lexer->lookahead)) {
            read_jump_statement_end_extent(lexer,
                                           no_comment_close_ahead(scanner));
            lexer->result_symbol = JUMP_STATEMENT_END;
            return true;
        }
    }

    // Same-line tokens: SUBSCRIPT_OPEN (§5.10) and COMMAND_NAMED_KEY
    // (§5.5). Both skip only spaces, tabs and line escapes, the
    // same-line whitespace of Groovy's lexer; at a newline we fall
    // through to the branches below. Their first characters (`[` vs. an
    // identifier start) are disjoint, so one whitespace skip serves
    // both. STATEMENT_PAREN (§6.18) shares the skip to turn down a `(`
    // on the line, which leaves the branches below a `(` only after a
    // line break. NOT_IN is valid exactly after an expression. Only rules
    // that may open a statement take STATEMENT_PAREN, so after an
    // expression it is valid exactly where a statement may start: inside
    // parentheses and brackets, where Groovy's lexer drops line breaks, it
    // is not, and the `(` on a later line opens a call's arguments there.
    bool wants_subscript = valid_symbols[SUBSCRIPT_OPEN];
    // §6.17 — after the `}` of the block that is the body of `if`, `else`,
    // a loop or an arrow case, NOT_IN is valid, as the braces may be a
    // closure, but the block is the reading that its weight ranks first,
    // and a reparse that reused the body lexes a `(` after it at a
    // statement's start. BODY_BLOCK_END, never emitted, is valid there, and
    // the `(` is read as at a statement's start: a lambda may open it on any
    // line (`if (c) { a() }` then `(x) -> x`).
    bool expression_ends = valid_symbols[NOT_IN]
        && !valid_symbols[BODY_BLOCK_END];
    bool wants_statement_paren = valid_symbols[STATEMENT_PAREN]
        && expression_ends && !valid_symbols[ERROR_SENTINEL];
    // §6.18 — where no expression ends, the same `(` is read as far, so
    // that its token depends on the same text in both states.
    bool wants_paren_extent = valid_symbols[STATEMENT_PAREN]
        && !expression_ends && !valid_symbols[ERROR_SENTINEL];
    bool wants_named_key = valid_symbols[COMMAND_NAMED_KEY];
    // §6.15 — TYPE_START is never emitted; it is valid where a type may
    // start, which narrows ARRAY_VALUE_START there.
    bool wants_array = valid_symbols[ARRAY_VALUE_START]
        && !valid_symbols[ERROR_SENTINEL];
    bool type_may_start = valid_symbols[TYPE_START];
    if ((wants_subscript || wants_named_key || wants_statement_paren)
        && !valid_symbols[ERROR_SENTINEL] && !body_continues) {
        if (!skip_same_line_space(lexer, true)) {
            return false;
        }
        // SPECIFICATION.md §5.10 — a `[` on the current line can continue
        // the expression (subscript or array type). After a newline the
        // in-grammar `[` starts a list or map literal instead.
        if (wants_subscript && lexer->lookahead == '[') {
            lexer->advance(lexer, false);
            // §6.23 — the end is marked, as the branch above may have
            // marked one before the spaces.
            lexer->mark_end(lexer);
            lexer->result_symbol = SUBSCRIPT_OPEN;
            return true;
        }
        if (wants_statement_paren && lexer->lookahead == '(') {
            return false;
        }
        // An identifier is either the key or, once consumed, not something
        // the code below could start (a comment or slashy string), so stop
        // here. Falling through after the scan advanced past `b` in
        // `foo a, b` and a newline made the comment branch emit `b` as a
        // line_comment, since mark_end had already been called.
        // The keywords of §6.7 and §6.13 can be valid here too, after a
        // statement-start identifier (`foo` then `record R(…)`). The key
        // wins on the same line: `foo record: 1` is a named argument,
        // while a declaration there is not valid Groovy. A keyword on the
        // next line is not reached here, since a newline is not skipped.
        if (wants_named_key && is_identifier_start(lexer->lookahead)) {
            return scan_command_named_key(lexer, valid_symbols, wants_array,
                                          type_may_start,
                                          no_comment_close_ahead(scanner));
        }
    }

    // SPECIFICATION.md §6.6 — line / block / groovydoc comments
    // and §6.2 slashy strings. Both start with `/`; combining their
    // dispatch keeps the in-grammar lexer from partially consuming
    // the leading `/`. Leading whitespace is skipped via
    // advance(true) so the scanner gets a chance to fire after the
    // in-grammar `\s` SKIP rule — without this, tree-sitter's lexer
    // eats `\n`, does NOT re-invoke the external scanner, then
    // fails on the following `/` and consumes it through error
    // recovery. The contextual keywords (§6.7, §6.13) and the receiver
    // of a call that is a value or a statement (§6.14) share the
    // whitespace skip and are dispatched on their first letter before the
    // `/` branches. A value may start on the line after its `=`, and a
    // statement on the line after the one before it, so those receivers
    // are looked for past line breaks too.
    bool wants_comment = valid_symbols[LINE_COMMENT]
        || valid_symbols[BLOCK_COMMENT]
        || valid_symbols[GROOVYDOC_COMMENT];
    bool wants_slashy = valid_symbols[SLASHY_STRING_START];
    bool wants_record = valid_symbols[RECORD_KEYWORD];
    bool wants_trait = valid_symbols[TRAIT_KEYWORD];
    bool wants_sealed = valid_symbols[SEALED_KEYWORD];
    bool wants_value_start = valid_symbols[COMMAND_VALUE_START]
        && !valid_symbols[ERROR_SENTINEL];
    // §6.21 — DECLARATION_START is valid where a statement may start and
    // after its annotations, and the same first name decides it.
    bool wants_statement_start = (valid_symbols[COMMAND_STATEMENT_START]
                                  || valid_symbols[DECLARATION_START])
        && !valid_symbols[ERROR_SENTINEL];
    bool wants_cast_close = valid_symbols[CAST_CLOSE]
        && !valid_symbols[ERROR_SENTINEL];
    bool wants_not_in = valid_symbols[NOT_IN]
        && !valid_symbols[ERROR_SENTINEL];
    // §6.17 — NOT_IN is valid only after an expression, where a `(` opens a
    // call's arguments (`foo(a) -> b`, an `ERROR`). A lambda may start there
    // only as the next statement, which is left unread: reading the
    // arguments of every such call would cost more than the rare lambda
    // statement after an expression is worth (`foo()` then `(a) -> a` on the
    // next line, which Groovy also reads as a call).
    bool wants_lambda = valid_symbols[LAMBDA_START]
        && !valid_symbols[ERROR_SENTINEL] && !expression_ends;
    bool wants_bare_lambda = valid_symbols[BARE_LAMBDA_START]
        && !valid_symbols[ERROR_SENTINEL];
    if (!wants_comment && !wants_slashy && !wants_record && !wants_trait
        && !wants_sealed && !wants_value_start && !wants_statement_start
        && !wants_array && !wants_cast_close && !wants_not_in && !wants_lambda
        && !wants_bare_lambda && !wants_paren_extent
        && !valid_symbols[MULTI_ASSIGNMENT_COMMA]
        && !valid_symbols[VAR_KEYWORD] && !valid_symbols[VAR_NAMES_KEYWORD]
        && !valid_symbols[YIELD_KEYWORD] && !valid_symbols[IN_KEYWORD]
        && !valid_symbols[AS_KEYWORD]) {
        return false;
    }

    // A line escape is whitespace too, so a comment, a slashy string or
    // a keyword may follow one.
    bool crossed_line = false;
    while (is_whitespace(lexer->lookahead) || lexer->lookahead == '\\') {
        if (lexer->lookahead == '\\') {
            if (!skip_same_line_space(lexer, true)) {
                return false;
            }
        } else {
            crossed_line = crossed_line || is_line_break(lexer->lookahead);
            lexer->advance(lexer, true);
        }
    }
    // A letter can never open a comment or a slashy string, so every
    // path out of the keyword branches returns. On `false` the runtime
    // rewinds to the token start and the in-grammar lexer reads the
    // word as an identifier.
    bool no_close_ahead = no_comment_close_ahead(scanner);
    // §6.14 — a statement's first name; it decides the keywords and the
    // array type below as well.
    if (wants_statement_start && !body_continues
        && is_identifier_start(lexer->lookahead)) {
        // §6.21 — after `def` the token is valid where no call may start,
        // and the type follows the token rather than standing here, so
        // TYPE_START is not valid; the type may start all the same
        // (`def String[] a`).
        bool declaration_type = type_may_start
            || (valid_symbols[DECLARATION_START]
                && !valid_symbols[COMMAND_STATEMENT_START]);
        return scan_statement_start(lexer, valid_symbols, wants_array,
                                    declaration_type, no_close_ahead);
    }
    if (lexer->lookahead == 'r' && wants_record) {
        if (!consume_keyword(lexer, "record")
            || !scan_record_header(lexer, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = RECORD_KEYWORD;
        return true;
    }
    if (lexer->lookahead == 't' && wants_trait) {
        if (!consume_keyword(lexer, "trait")
            || !scan_trait_header(lexer, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = TRAIT_KEYWORD;
        return true;
    }
    if (lexer->lookahead == 's' && wants_sealed) {
        // §6.15 — `short` is the one element type name that shares a
        // first letter with these keywords, and a statement may start
        // with `short[].class` where `sealed` may start too. The array
        // token is empty and ends here; the keyword moves the end again.
        lexer->mark_end(lexer);
        advance(lexer);
        if (lexer->lookahead == 'h') {
            if (!wants_array || !consume_keyword(lexer, "hort")
                || !array_dimensions_follow(lexer, type_may_start,
                                            no_close_ahead)) {
                return false;
            }
            lexer->result_symbol = ARRAY_VALUE_START;
            return true;
        }
        if (!consume_keyword(lexer, "ealed")
            || !scan_sealed_target(lexer, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = SEALED_KEYWORD;
        return true;
    }
    // §6.16 — `var`, `yield`, `in` and `as`. `int` is the one element
    // type name (§6.15) that shares a first letter with them, and it may
    // start a statement where `in` may follow the one before, so it is
    // decided here too; the array token is empty and ends here. Where a
    // value or a bare lambda may start as well, any name may, and the words
    // are decided in `scan_value_start` once the whole name is read: a name
    // that only starts like one of them (`v -> v`, `value 1`) is not turned
    // down after its first letters.
    if (contextual_word_may_start(lexer->lookahead, valid_symbols)
        && !wants_value_start && !wants_bare_lambda) {
        lexer->mark_end(lexer);
        const char *word = read_contextual_word(lexer);
        if (word == NULL) {
            return false;
        }
        unsigned len = (unsigned)strlen(word);
        if (word_is(word, len, "int")) {
            if (!wants_array
                || !array_dimensions_follow(lexer, type_may_start,
                                            no_close_ahead)) {
                return false;
            }
            lexer->result_symbol = ARRAY_VALUE_START;
            return true;
        }
        lexer->mark_end(lexer);
        return scan_contextual_word(lexer, word, len, valid_symbols,
                                    no_close_ahead);
    }
    if (((wants_value_start || wants_bare_lambda)
         && is_identifier_start(lexer->lookahead))
        || (wants_array && is_element_type_start(lexer->lookahead))) {
        return scan_value_start(lexer, valid_symbols, wants_value_start,
                                wants_bare_lambda, wants_array, type_may_start,
                                no_close_ahead);
    }
    if (lexer->lookahead == '!' && wants_not_in) {
        // SPECIFICATION.md §6.12 — `!in` is an operator only before
        // whitespace or an opening bracket (GroovyLexer.g4 `NOT_IN`),
        // so that `a !inside` is `!` applied to a name. The character
        // after it is read and not taken: `a !in[1, 2]`.
        static const char not_in[] = "!in";
        for (const char *c = not_in; *c != '\0'; c++) {
            if (lexer->lookahead != *c) {
                return false;
            }
            advance(lexer);
        }
        int32_t next = lexer->lookahead;
        if (!is_whitespace(next) && next != '[' && next != '('
            && next != '{') {
            return false;
        }
        lexer->result_symbol = NOT_IN;
        return true;
    }
    if (lexer->lookahead == ')' && wants_cast_close) {
        // On `false` the runtime rewinds and the in-grammar lexer reads
        // a plain `)`. A cast that closes with one ranks below a
        // parenthesized expression.
        advance(lexer);
        switch (read_paren_close(lexer)) {
            case PLAIN_CLOSE:
                // A `)` that ends its line before a line that opens with
                // `(`, with no bracket around both, closes no cast
                // (`next_line_paren_ends_cast`), and that reading ends
                // here. TYPE_ONLY_CLOSE, never emitted, is valid after an
                // array or generic type, which is no expression: the cast
                // of one is the only reading of its parentheses, and stays
                // (`(List<String>)` then `(x)`).
                if (!valid_symbols[TYPE_ONLY_CLOSE]
                    && next_line_paren_ends_cast(lexer, no_close_ahead)) {
                    lexer->result_symbol = RELATIONAL_CLOSE;
                    return true;
                }
                return false;
            case OPERAND_CLOSE:
                // §6.17 — a `(` after the `)` opens a lambda where the `)`
                // closed a cast (`(Runnable) () -> { }`) and a call's
                // arguments where it closed an expression, and only the
                // first reading's scan reads on to the `->`. The reading
                // that survives may be the call (`(A) () x`), whose `(`
                // read nothing, so read as far here: both readings take
                // this `)`, and an edit that adds or removes the `->` makes
                // the runtime lex it again and fork anew.
                if (lexer->lookahead == '(') {
                    bool closed = false;
                    bool in_string = false;
                    lambda_parameters_follow(lexer, no_close_ahead, &closed,
                                             &in_string);
                }
                lexer->result_symbol = CAST_CLOSE;
                return true;
            case NAME_CLOSE:
                lexer->result_symbol = valid_symbols[CAST_WORD_CLOSE]
                    ? CAST_WORD_CLOSE
                    : CAST_CLOSE;
                return true;
            default:
                // Also where only a cast's type takes the `)`, which no
                // rule takes this token in: that reading ends here.
                lexer->result_symbol = RELATIONAL_CLOSE;
                return true;
        }
    }
    if (lexer->lookahead == '(' && wants_statement_paren) {
        // On `false` the runtime rewinds and the in-grammar lexer reads
        // the `(` of a call's arguments.
        if (!parentheses_open_statement(lexer, no_close_ahead)) {
            return false;
        }
        lexer->result_symbol = STATEMENT_PAREN;
        return true;
    }
    if (lexer->lookahead == '(' && wants_lambda) {
        // The token is empty and ends here. On `false` the runtime rewinds
        // and the in-grammar lexer reads the `(`.
        lexer->mark_end(lexer);
        bool closed = false;
        bool in_string = false;
        if (!lambda_parameters_follow(lexer, no_close_ahead, &closed,
                                      &in_string)) {
            // §6.18 — STATEMENT_PAREN is valid only where a statement may
            // start, and a lambda may start there too (every external
            // scanner state with STATEMENT_PAREN has LAMBDA_START), so
            // where no expression ends this is the one branch that reads
            // that `(` as far as where one does. At a dollar-slashy string
            // that reading has ended.
            if (wants_paren_extent && crossed_line && !in_string) {
                read_parentheses_extent(lexer, closed, no_close_ahead);
            }
            return false;
        }
        lexer->result_symbol = LAMBDA_START;
        return true;
    }
    if (lexer->lookahead == '}' && valid_symbols[STATEMENT_PAREN]
        && !valid_symbols[ERROR_SENTINEL]) {
        read_lambda_body_end_extent(lexer, no_close_ahead);
        return false;
    }
    // §6.16 — the name `in` where its keyword is not valid, as after a
    // type that a reparse reused (`for (x in )`, where `x` was reduced to a
    // type while no expression followed `in`): read as far as where the
    // keyword is decided, so that the name's token depends on the same
    // text in both states, and an edit after it makes the runtime lex it
    // again and break the reused type down. No branch above took the name.
    if (lexer->lookahead == 'i' && !valid_symbols[ERROR_SENTINEL]) {
        if (consume_keyword(lexer, "in")) {
            expression_follows(lexer, false, no_close_ahead);
        }
        return false;
    }
    // §6.20 — the first comma of a multi-assignment, or a form feed or
    // vertical tab before it, which the loop above leaves. No branch above
    // takes those characters, and testing the token's validity only here,
    // rather than with the flags above on every scan, is cheaper.
    if (lexer->lookahead != '/') {
        return valid_symbols[MULTI_ASSIGNMENT_COMMA]
            && !valid_symbols[ERROR_SENTINEL]
            && scan_multi_assignment_comma(lexer, no_close_ahead);
    }

    // §6.23 — after `return`, `break` or `continue` a line comment, and a
    // block comment that only spaces and tabs follow before a line break
    // or the end of input, end the statement before them, as Groovy's
    // lexer reads either as the line break (`break // c`):
    // JUMP_STATEMENT_END is emitted here, and the comment by the next call,
    // outside the statement. The token ends at the keyword, where the
    // branch above marked it. After the marker of `jump_continues` the
    // comment is read without that decision.
    bool jump_end = valid_symbols[JUMP_STATEMENT_END]
        && !valid_symbols[ERROR_SENTINEL] && !jump_continues;
    // UNTERMINATED_COMMENT is empty and ends here, or after the keyword
    // where JUMP_STATEMENT_END is valid. Every other token of this branch
    // moves the end again.
    if (!jump_end) {
        lexer->mark_end(lexer);
    }
    advance(lexer);
    // Dispatch on the char after `/`:
    //   `/`  → line comment
    //   `*`  → block / groovydoc comment
    //   `=`  → augmented assign, in-grammar handles it
    //   else → slashy string (when valid_symbols allow)
    int32_t next = lexer->lookahead;
    if (next == '/') {
        if (!valid_symbols[LINE_COMMENT]) {
            return false;
        }
        advance(lexer);
        // EOF-bounded loop: lexer->eof() guarantees termination on
        // unterminated input. Do not tighten without preserving the
        // EOF guard. A lone `\r` ends the line too, as in Groovy's
        // lexer (`LineTerminator`), so the comment stops before it.
        while (!lexer->eof(lexer) && !is_line_break(lexer->lookahead)) {
            advance(lexer);
        }
        if (jump_end) {
            read_jump_statement_end_extent(lexer, no_close_ahead);
            lexer->result_symbol = JUMP_STATEMENT_END;
            return true;
        }
        lexer->mark_end(lexer);
        lexer->result_symbol = LINE_COMMENT;
        if (scanner != NULL) {
            scanner->slashy_after_comment_newline = false;
        }
        read_deferred_extent(lexer, body_continues, no_close_ahead);
        return true;
    }
    if (next == '*') {
        if (!wants_comment || no_comment_close_ahead(scanner)) {
            return false;
        }
        advance(lexer);
        bool is_groovydoc = false;
        // `/**/` closes at its second `*`, a Groovydoc comment.
        bool closed = false;
        if (lexer->lookahead == '*') {
            advance(lexer);
            is_groovydoc = true;
            if (lexer->lookahead == '/') {
                advance(lexer);
                closed = true;
            }
        }
        // EOF-bounded loop: a `/* …` without a closer terminates
        // here via lexer->eof(), not by going past end-of-input.
        while (!closed && !lexer->eof(lexer)) {
            if (lexer->lookahead == '*') {
                advance(lexer);
                if (lexer->lookahead == '/') {
                    advance(lexer);
                    closed = true;
                }
            } else {
                advance(lexer);
            }
        }
        if (closed) {
            // §6.23 — the end of the token before the comment is still at
            // the keyword. Where only spaces and tabs follow the comment
            // on its line, Groovy's lexer reads it as the line break
            // (`isFollowedByWhiteSpaces`), and the statement ends before
            // it. Where something else follows them, the comment's own
            // end is passed: the empty marker sets `jump_continues`, and
            // the next call reads the comment without deciding again.
            if (jump_end) {
                bool spaces = false;
                while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                    advance(lexer);
                    spaces = true;
                }
                if (lexer->eof(lexer) || is_line_break(lexer->lookahead)) {
                    read_jump_statement_end_extent(lexer, no_close_ahead);
                    lexer->result_symbol = JUMP_STATEMENT_END;
                    return true;
                }
                // Without the scanner's memory the comment takes the
                // spaces after it.
                if (spaces && scanner != NULL
                    && valid_symbols[UNTERMINATED_COMMENT]) {
                    scanner->jump_continues = true;
                    lexer->result_symbol = UNTERMINATED_COMMENT;
                    return true;
                }
            }
            lexer->mark_end(lexer);
            if (is_groovydoc) {
                if (!valid_symbols[GROOVYDOC_COMMENT]) {
                    return false;
                }
                lexer->result_symbol = GROOVYDOC_COMMENT;
            } else {
                if (!valid_symbols[BLOCK_COMMENT]) {
                    return false;
                }
                lexer->result_symbol = BLOCK_COMMENT;
            }
            // A comment can swallow the newline before it. Keep that
            // fact only for a statement's next comment or slashy string;
            // inside parentheses a slash after the comment still divides.
            if (scanner != NULL) {
                bool after_newline = !body_continues
                    && valid_symbols[STATEMENT_PAREN]
                    && (crossed_line || scanner->slashy_after_comment_newline);
                scanner->slashy_after_comment_newline = false;
                if (after_newline) {
                    while (is_inline_space(lexer->lookahead)) {
                        advance(lexer);
                    }
                    if (lexer->lookahead == '/') {
                        advance(lexer);
                        scanner->slashy_after_comment_newline =
                            lexer->lookahead == '*'
                            || (lexer->lookahead != '='
                                && lexer->lookahead != '/'
                                && slashy_body_follows(lexer, false));
                    }
                }
            }
            read_deferred_extent(lexer, body_continues, no_close_ahead);
            return true;
        }
        // The runtime drops an empty extra token unless the state
        // changed with it. This one sets the flag, and the check above
        // keeps a second one from being emitted in the same state.
        if (scanner == NULL || !valid_symbols[UNTERMINATED_COMMENT]) {
            return false;
        }
        scanner->no_comment_close_ahead = true;
        lexer->result_symbol = UNTERMINATED_COMMENT;
        return true;
    }
    if (next == '=') {
        // `/=` is the augmented-assign operator. The in-grammar
        // lexer owns it. Returning false here causes tree-sitter
        // to rewind the cursor to before the leading `/`, so the
        // in-grammar lexer can match `/=` cleanly.
        return false;
    }
    // Slashy string opening `/` — we've consumed it; the body and
    // closing `/` are matched by the grammar's `slashy_string` rule
    // via regex tokens, so `$identifier` and `${expr}` segments can
    // be exposed as structured children. SPECIFICATION.md §5.4 /
    // §6.2 require a context-sensitive emit. Three pragmatic guards
    // keep `a / b` and similar division contexts from being
    // mis-tokenised as the opening of a slashy:
    //
    //   (1) Reject an empty body (the next char after the opening
    //       `/` is a closing `/`) — `//` is already a line comment
    //       and `/=` is already trapped above, so an empty slashy
    //       is excluded.
    //   (2) Reject a body that starts with whitespace — `a / b`
    //       has a space immediately after the operator `/`, but
    //       every legal slashy regex starts with a non-whitespace
    //       char.
    //   (3) Look ahead through the body to confirm there is a
    //       closing `/` on the same line. Without this, `a / b\nc`
    //       at end of file would emit a slashy opener and leave
    //       the body unterminated.
    // §6.2 — after an operand Groovy's lexer reads `/` as division
    // (GroovyLexer.g4 `isRegexAllowed`: after a name, a literal, `this`,
    // `)`, `]`, `}`, `++`, `--` or the end of a string). NOT_IN is valid
    // exactly where an expression ends, which on the same line is after
    // such a token, so the next statement or a call's first argument that
    // the grammar lets start there opens no slashy string: `x = w/2 + h/2`
    // was `x = w` and the string `/2 + h/`. Past a line break one may
    // start, as Groovy's lexer emits a newline token there (`x = a` and
    // `/re/` on the next line).
    if (!wants_slashy
        || (expression_ends && !crossed_line
            && (scanner == NULL || !scanner->slashy_after_comment_newline))) {
        return false;
    }
    // `lexer->mark_end` records the position where the slashy opener
    // token ends; advancing past it during the look-ahead does not extend
    // the token range as long as we do not call `mark_end` again.
    lexer->mark_end(lexer);
    if (!slashy_body_follows(lexer, !expression_ends)) {
        return false;
    }
    if (scanner != NULL) {
        scanner->slashy_after_comment_newline = false;
    }
    lexer->result_symbol = SLASHY_STRING_START;
    return true;
}
