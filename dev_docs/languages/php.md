# php

The PHP plugin (`crates/cgg-lang/src/plugins/php.rs`, 529 lines, `PhpPlugin`, id `"php"`) is a hand-written recursive tree-sitter walker (`PhpWalker`, php.rs:60-393) over the `tree-sitter-php` crate (`Cargo.toml:99`, `tree-sitter-php = "0.24"`, bound as `tree_sitter_php::LANGUAGE_PHP`, php.rs:31-33). Nothing is vendored. It extracts named functions, class/interface/trait/enum methods, PHP 8 attributes, `extends`/`implements`/`use Trait` bases, `namespace`/`use` imports, `require`/`include` statements, and every function/method/nullsafe/static call. It declares the signals `dyn_uses`, `attributes`, `impls` and `value_refs` (php.rs:21-29), although only `dyn_uses` and `attributes` are backed by extraction: it never sets `trait_impl_target`, `RefRecord::value_ref`, visibility, arity, locals or local types. There is no per-plugin resolver kind (`resolver_kind` does not exist in `cgg-lang`). PHP files go through the shared intra-file / cross-file pipeline plus one PHP-specific resolver, `PhpIndex` in `crates/cgg-resolve/src/libscope.rs:1876-2265`, called from `cross_file.rs:5500-5525`. `PhpIndex` resolves namespaces, `use` tables, `parent::`/`self::`/`static::`/`$this->`, inherited and trait members, and copies of a function narrowed by the `require`/`include` chain.

Fixtures are in `<scratch>/parity/php/`, directories `p1` … `p14`, with a JSON dumper at `dump.py`. They were run with `target/release/cgg` (0.11.0).

## 01. Detection & parsing

Supported:

- **Extensions** `.php` and `.phtml` (php.rs:15-17). Fixture p6 `view.phtml` gives `detected_via: extension:.phtml`. Matching ignores case: fixture p7 `UP.PHP` was analysed.
- **Shebang** `php` (php.rs:18-20). Fixture p6 `runme` (`#!/usr/bin/env php`, no extension) gives `detected_via: shebang:php`.
- **Grammar**: a single grammar, `LANGUAGE_PHP` (php.rs:32), which is the PHP-in-HTML grammar: text outside `<?php … ?>` is a `text` node. `LANGUAGE_PHP_ONLY` is not used, and `ts_language_for` is not overridden. In fixture p6 `view.phtml`, which mixes HTML with `<?= render_item($i) ?>`, the inline call was recorded and `render_item -> esc` resolved.
- **Depth skip**: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `too-deep` (`crates/cgg/src/lib.rs:155,451-454`). Fixture p6 `deep.php` (5000 nested parens) gives `file_skipped … {'kind': 'too-deep', 'detail': 4000}`.
- **Minified files**: `MINIFIED_LINE_BYTES` = 2000 (`crates/cgg/src/lib.rs:159,482`) only disables `locals::collect`. PHP has no locals spec, so the threshold changes nothing for PHP. Fixture p6 `min.php` (one 4 KB line) was analysed, and its `m1 -> m2` edge was found.
- **Parse errors**: the file is still extracted, with `parse_status` set to `error` (shared behaviour, `crates/cgg/src/lib.rs`).

Not supported:

- **Content detection**: there is none. A file with no extension that starts with `<?php` and has no shebang is skipped as `unknown-extension` (fixture p7 `noext`). `crates/cgg-lang/src/detect.rs` has no PHP header heuristic (grepping it for `php` finds nothing).
- **Other PHP extensions are not detected**: `.inc`, `.php5`, `.module` (Drupal), and likewise `.php3`/`.php4`/`.php7`/`.phps`/`.ctp`/`.install`/`.theme`/`.engine`. Fixture p6 shows `other.inc`, `legacy.php5` and `mod.module` as `unknown-extension`. Drupal's `.module`/`.install` hook files are therefore invisible, which compounds the `drupal` rule gap (§10).
- **No re-parse or fallback**: unlike C#'s `#if` blanking, a PHP file is never re-parsed.
- **Blade/Twig templates** (`.blade.php` is detected as PHP, but `@directive` syntax is HTML text to the grammar) are not modelled. This is unverified beyond the grammar choice.

## 02. Callables extracted

Every definition goes through `record_def` (php.rs:224-245). The qualified name is the class scope joined with `::`, or the bare name (`qn`, php.rs:76-82). The namespace is never part of a callable's qualified name. `signature_hint = extract_signature(node text)`, `attributes = attrs::collect` (php.rs:241), `base_types` = bases of the innermost enclosing class (php.rs:242), and `visibility` is always empty (php.rs:239).

Supported:

- **Top-level function** `function f() {}` → simple `f`, qn `f`, `DefVariant::FreeFunction`, kind `function` (php.rs:149-164). Fixture p1: `helper_fn`, `never_called`, `top`, `fmt`.
- **Function inside a namespace** `namespace App\Util; function fmt() {}` → qn `fmt`, with no namespace prefix. The namespace is applied only inside `PhpIndex` (libscope.rs:2010-2050, `php_join(ns, simple_name)`). Fixture p1: `fmt`.
- **Class method** `class User { public function run() {} }` → qn `User::run`, `DefVariant::InherentMethod`, kind `method` (php.rs:165-175). Static methods are the same: `Helper::stat` in fixture p1.
- **Interface method** `interface Named { public function name(): string; }` → `Named::name`, a plain method (php.rs:94-97). Fixture p1.
- **Abstract method** `abstract public function name();` → `Base::name`, a plain method. It is not tagged `abstract`, so `stub_ids` (cross_file.rs:1161-1186, which keys on an `abstract`/`abstractmethod` attribute or a Protocol/ABC base) never sees it. Fixture p1.
- **Trait method** `trait Greets { function greet() {} }` → `Greets::greet` (php.rs:94-97). Fixture p1.
- **Enum method** `enum Suit: string { public function label() {} }` → `Suit::label`. Fixture p1.
- **Constructor / destructor / magic methods** `__construct`, `__destruct`, `__get`, `__toString`, `__call`, `__callStatic`, `__invoke` are recorded as ordinary methods: kind `method`, not `Constructor`/`Destructor` (fixtures p1 and p2, `Ctor::__construct` kind `method`).
- **Constructor property promotion** `__construct(private Helper $h)`: the constructor is a callable, and the promoted parameter's type is read as a parameter type (type_hints.rs:3459-3464, `private ?Foo $e` → `$e: Foo`).
- **Nested named function** `function outer() { function nested() {} }` → a free function `nested`, qn `nested` with no owner (fixture p2 `inner_fn`, fixture p10 `nested`). This matches PHP semantics, where nested functions are global once defined.
- **Conditional function** `if (!function_exists('f')) { function f() {} }`, in both brace and `: … endif;` forms → `DefRecord::conditional = true` (`is_conditional`, php.rs:395-411). Test: `a_guarded_function_is_conditional` (php.rs:513).
- **Same-name copies in one file**: both are callables (fixture p1 `lib/functions.php`, two `helper_fn` nodes).
- **Class FQN records**: a top-level class/interface/trait/enum pushes a `ClassDecl { class_qn: "<ns>\\<Name>", base_types: [] }` (php.rs:102-116). Test: `grouped_use_and_class_fqns` (php.rs:494).

Not supported or partial:

- **Closures, arrow functions and static closures** (`function ($a) {…}`, `fn($b) => …`, `static fn() => …`) are not callables. Calls inside them are attributed to the enclosing named callable. In fixture p1, `helper_fn` called inside `$fn = function…` and `$arrow = fn…` shows up as `User::run -> helper_fn` (L41, L42). At file top level they have no enclosing callable: the routes closure in fixture p2 `routes/web.php` produced no node. php.rs has no arm for `anonymous_function`/`arrow_function` (php.rs:85-210).
- **Anonymous classes** `new class { public function anon() {} }`: the methods are attributed to the *enclosing* class, because the scope is not pushed for a nameless class (php.rs:139-145). Fixture p2 yields the qn `Factory::anon`, which is the wrong owner.
- **Property hooks (PHP 8.4)**, **enum cases** and **class constants** are not callables. php.rs has no arm for them, and the fixture p1 enum case `Hearts` produced no node.
- **Braced namespaces** `namespace A { … }` are mis-recorded (see §04). Callables are still extracted with their class-scoped qns, as fixture p4 shows.
- **Namespace-qualified callable names**: two classes `App\A\User` and `App\B\User` both yield `User::m` qns. Only `PhpIndex` distinguishes them, by file.
- **Overloads and clauses**: not applicable, because PHP has no overloading. Same-name re-declarations are handled as copies (§07).

## 03. Call-site forms

All calls go through `record_call` (php.rs:306-392), reached from `function_call_expression`, `member_call_expression`, `nullsafe_member_call_expression` and `scoped_call_expression` (php.rs:198-206).

Supported:

- **Bare call** `helper_fn(1)` → name `helper_fn`, empty receiver (php.rs:311-315). Fixture p1 `top -> helper_fn`.
- **Fully-qualified function call** `\A\use_base($x)` → name `\A\use_base`. `PhpIndex` resolves names containing `\` through `class_fqn` (libscope.rs:2230-2234). Fixture p5 gives `f -> use_base` high, but the same site also yields an `<external>::\A\use_base` leaf (see §16).
- **Method call** `$x->help()` → name `help`, receiver `$x` (php.rs:349-365). Fixture p1 L27.
- **`$this->m()`** → receiver rewritten to `this` (php.rs:369-373). Test: `this_receiver_is_spelled_this` (php.rs:456).
- **Nullsafe** `$c?->send()` → a member call. Integration test `null_conditional_calls_are_recorded_in_dart_and_php` (crates/cgg/tests/resolve.rs:3196). Fixture p1 `<external>::Client::send` 2x.
- **Static / scoped call** `Helper::stat()`, `\App\Util\Helper::stat()`, `self::m()`, `static::m()`, `parent::m()` → the name comes from the `name` field and the receiver from `scope` (php.rs:311-315, 361-364). Fixture p1: `Helper::stat` high (both forms), `self::make` and `static::make` resolve to `Base::make`, `parent::boot` resolves to `Base::boot`.
- **Parenthesised receiver / construction receiver** `($f)->m()` and `(new \App\Foo())->m()` are peeled by `crate::receiver::peel`. A `new` receiver becomes the class short name with `typed_receiver = true` (php.rs:346-365; receiver.rs:343-345 strips the namespace; test `php_parenthesized_new`, receiver.rs:588). Fixture p1 L39 `(new Helper())->help()` gives `Helper::help` high.
- **Case-insensitive callee** `$s->UNIQUEM()` / `helper_case()` vs `HELPER_CASE`: `PhpIndex` lowercases names (libscope.rs:1916-1923, 2149-2150). Fixture p3 `$s->UNIQUEM()` gives `Svc::uniqueM` high. Fixture p2 `helper_case()` gives `HELPER_CASE` high.
- **First-class callable syntax** `$this->viaVar(...)`, `self::create(...)` is recorded as an ordinary call edge, not a reference (fixture p2 `Factory::firstClass -> Factory::viaVar`, `-> Factory::create`).
- **Registrar arguments** (§10): when the callee's last segment is a registrar verb of some PHP rule (`ExtractCtx::is_registrar_verb`, registrar.rs:424-440), the arguments are captured as string refs (`'Controller@method'`, `'Class::method'`, `'fn_name'`) or value refs (`[Controller::class, 'index']`, `[$this, 'm']`, `array('C','m')`) (php.rs:387-391). PHP's `argument` wrapper is unwrapped (registrar.rs:295-312). Fixtures p2 and p8.

Not supported or partial:

- **`new Foo(...)` as a call**: `object_creation_expression` is never recorded as a call site (php.rs has no arm for it), so no edge reaches `__construct` from a construction. `constructor_names("php") = ["__construct"]` (cross_file.rs:387) is unreachable because PHP never emits a bare `Foo(…)`. Fixture p2: `Factory::create` does `new Ctor()` and there is no `-> Ctor::__construct` edge. Fixture p14 shows the same.
- **Variable functions and callable invocation** `$fn(1)`, `$inv()`: recorded with name `$fn` / `$inv`, which then become `<external>::$fn` (fixture p1). `call_operator_names("php") = ["__invoke"]` (cross_file.rs:397) needs `local_types`, which PHP never records, so `$inv()` never reaches `Helper::__invoke` (fixture p1).
- **Dynamic method names** `$this->$m()` and `$this->{$name}()` are recorded as name `$m` and become `<external>::this::$m` (fixture p1).
- **Callables as values** `array_map('helper_fn', …)`, `array_map([$this, 'name'], …)`, `call_user_func([$this,'name'])`, `Closure::fromCallable('f')`, `strlen(...)`: no edge and no reference unless the callee is a registrar verb (fixture p1: no edge to `User::name`; fixture p1 with `--reference-edges` gives 0 `ref` edges). php.rs never calls `registrar::capture_value_refs` or `capture_value_position` (registrar.rs:517, 563).
- **Chained calls on a call result** `Svc::make()->uniqueM()`, `$s->self_chain()->uniqueM()`: the receiver is the call text and becomes external (fixture p3 `<external>::Svc::make()::uniqueM`).
- **Arity**: `RefRecord::arity` is never set, and PHP is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583).
- **Paren-less calls, explicit generics, operator overloading**: not applicable to PHP.
- **`require`/`include` written as a function call** (`require_once('x.php')`) becomes an import, not a call (php.rs:320-344). The grammar normally yields `require_once_expression`, which is handled at php.rs:179-197.

## 04. Imports & module resolution

Supported:

- **`namespace App\Http;`** (semicolon form) → `ImportRecord { kind: "package", path }`, and it sets the walker's namespace (php.rs:249-270). `PhpIndex` keeps a per-file `(byte, namespace)` list (`php_ns_at`, libscope.rs:1908-1914) and every ancestor namespace (libscope.rs:1933-1942).
- **`use A\B\C;`, `use A\B\C as D;`** → `kind: "use"`, with alias (php.rs:289-302). `PhpIndex.uses` maps the lowercased alias, or the last segment, to the lowercased FQN (libscope.rs:1944-1955).
- **Grouped use** `use A\{B, C as D};` is expanded per item (php.rs:271-283). Test `grouped_use_and_class_fqns` (php.rs:494). Fixture p5 `use A\{Ci, Base}` gives `Ci::area` high.
- **`use function ns\f;` / `use const ns\X;`**: the `function`/`const` keyword is stripped (php.rs:255-256), and the record is a `use`. A bare `f()` looks up the `use` table first (libscope.rs:2224-2226). Fixture p1 `use function App\Util\fmt` gives `User::name -> fmt` high. Fixture p5 `use function A\use_shape` gives `f -> use_shape` high.
- **Bare function resolution order** (libscope.rs:2208-2246): the `use function` import, then `<current namespace>\f`, then global `f`, all case-insensitive. A function name the tree declares only in *other* namespaces is `External` (libscope.rs:2240-2245). Test: `php_bare_call_reaches_functions_never_methods` (cross_file.rs:12106).
- **Class resolution** `X::m()` (libscope.rs:2137-2151, `class_fqn`): a leading `\` makes it absolute; otherwise the head is looked up in the `use` table, falling back to the current namespace. A class whose namespace is not declared in the tree is `ClassLookup::External` → `Scoped::External` (libscope.rs:2164-2170, 2155-2161). Test: `php_namespaces_and_use_tables_decide_the_class` (cross_file.rs:13713). Fixture p1 `Client::connect()` (`use Vendor\Lib\Client`) gives `<external>::Client::connect`. Fixture p5 `Thing::run()` gives external.
- **`require`/`require_once`/`include`/`include_once`** → `kind: "php-require"`, path = argument text, at most 400 bytes (php.rs:179-197). A function-call spelling uses the callee as the kind (php.rs:320-344). `PhpIndex.requires` keeps each target's *basename* (libscope.rs:1994-2010), and `one_copy` walks the include closure to depth 8 by basename (libscope.rs:2067-2088). Test: `php_unconditional_definition_beats_guarded_fallbacks` (cross_file.rs:13960). Fixture p10 `require_once 'inc/cache.php'` gives `run -> get_multi` (inc/cache.php) high.
- **Framework detection** reads these imports. Without them the Symfony/Laravel/CodeIgniter rules would be inert (comment at php.rs:86-90). Integration test `detect_prefixes.rs:44` builds a PHP `use {prefix}\Thing;` per rule.

Not supported or partial:

- **Braced namespaces** `namespace A { … }`: `record_use` takes the *whole node text* (php.rs:250). When the block exceeds 400 bytes the record is dropped (php.rs:260-262). A shorter block becomes a garbage namespace, mis-expanded by the `{…}` group-use logic (php.rs:271-283). Either way the classes and functions in it are not under the right namespace. Fixture p4 (a large braced file): `\A\use_base(…)` and `Square::nope()` gave external only, and `use A\Sq as Square` + `new Square()` did not resolve. Fixture p5 `braced.php` (small block): `\C\cf()` → `<external>::\C\cf`, with no edge, while the semicolon form in the same run resolved `\A\use_base`.
- **`FileFacts::packages`** is never written. The namespace lives only in `ImportRecord{kind:"package"}`, and php.rs never pushes `facts.packages`.
- **`heads.rs` / `WorkspaceVocab`**: PHP is not handled there (heads.rs language arms: python, go, java/kotlin, …). External-module refusal for PHP comes only from `PhpIndex` (`Scoped::External`).
- **`not-in-imported-module`**: never produced for PHP. `PhpIndex::resolve` returns only `Bound`, `External`, `Ambiguous` or `None` (libscope.rs:2201-2265).
- **Composer**: `composer.json` (PSR-4 autoload, package name) is not read. `manifest.rs` has no PHP reader, and namespace→directory mapping is not used. Membership comes only from the namespaces the tree declares.
- **Include path matching uses the basename only**: `require 'a/functions.php'` reaches every `functions.php` in the tree (libscope.rs:2004-2009, 2074). Dynamic paths (`__DIR__ . '/x.php'`) work only because the basename is cut from the last `/`, `'` or `"`.
- **The include chain does not narrow two copies in one file**: fixture p1 has two `helper_fn` definitions in `lib/functions.php` (one conditional). `top -> helper_fn` binds both at medium, because `one_copy` returns `Bound` for a single file (libscope.rs:2059-2063) and never applies the conditional rule there.
- **Re-exports, `class_alias()`, and `spl_autoload_register`**: not modelled (no code in php.rs or libscope.rs).

## 05. Type inference & receiver typing

Supported:

- **Typed parameters** `function run(Helper $x) { $x->help(); }` → `$x: Helper`, read from `signature_hint` by the shared "Type name" parser, with the `$` kept in the variable name (type_hints.rs:1465-1470, which notes that the C++ parser rejects `ContainerBuilder $container`; test at type_hints.rs:3459-3464). Fixture p1 L27 gives `Helper::help` high, and fixture p3 L9 `$s->shared()` gives `Svc::shared` high.
- **Nullable / union / promoted** `?Foo $a`, `Foo|null $b`, `null|Foo $c`, `private ?Foo $e` → `Foo`. `Foo|Bar` gives no type (type_hints.rs:3459-3464). `NULL_TYPE_MEMBERS` has a PHP entry `["null","false","void"]` (stdlib.rs:135), and `declared_type("php", …)` is tested at annotation.rs:172-201.
- **Return types** `function f(): ?Foo` → `Foo` (`extract_return_type`, test at type_hints.rs:3506). These are recorded in the return-type map. No PHP fixture showed a return type typing a receiver (see below).
- **Construction receiver** `(new Foo())->m()` → `typed_receiver` (§03). Fixture p1 L39.
- **Typed receiver of an external class** `?Client $c; $c->send()` → `<external>::Client::send` through `PhpIndex` (fixture p1).
- **Locals** use the shared file-wide, last-write-wins map. PHP records no `scope_byte` (comment at type_hints.rs:720-724; test `locals_outside_cpp_keep_the_file_wide_lookup`, type_hints.rs:3268).

Not supported:

- **Local assignment typing** `$y = new Helper(); $y->help();` gives `<external>::$y::help` (fixture p1 L38; fixture p3 `$local`; fixture p14 `$o`). The plugin pushes no `local_types` (php.rs has no such call).
- **Typed properties / fields** `private Helper $helper; $this->helper->help()` gives `<external>::$this->helper::help` (fixture p1 L29; fixture p3 `private Svc $svc`). Promoted properties behave the same (`$this->h->help()`, fixture p1 L46). There are no `class_fields`.
- **Return-type propagation into locals and chains** `$x = Svc::make(); $x->uniqueM()` and `Svc::make()->uniqueM()` both give external (fixture p3 L11, L16).
- **Docblock types** `/** @var Svc */`, `@param`, `@return`: not read (fixture p3 `$doc`). PHPStan/Psalm generics are not read either.
- **`instanceof` narrowing, `assert($x instanceof Foo)`, and `static`/`self` return types for fluent chains**: not modelled.
- **Closure parameter typing** (`closure_types.rs`, no PHP), **implicit receivers**, and **weak receiver types**: none for PHP.
- **`CAST_INTRINSICS` / `CONSTRUCTING_METHODS` / `MAYBE_TYPE_WRAPPERS` / `UNION_TYPE_CONSTRUCTORS` / `TOP_TYPES` / `CALLBACK_TYPE_CONSTRUCTORS`** have no PHP entries (stdlib.rs:35-46, 58, 104-122, 125, 143-155). `(Foo) $x` casts exist only for scalars in PHP and are not applicable.

## 06. Inheritance, dispatch & overloading

Supported:

- **Bases on methods**: `extends`/`implements` from `attrs::base_types`, plus `use T1, \N\T2;` traits in the class body, are appended as bases (php.rs:117-138). Test: `used_traits_are_bases` (php.rs:449). `PhpIndex.bases` keeps them per `(file, class)` (libscope.rs:1980-1984).
- **`$this->m()`, `self::m()`, `static::m()`** resolve through the calling class, its bases and its traits (`class_method`, depth 8, libscope.rs:2162-2196). The result is certain only when the method is the class's own and the class has a single file. An inherited hit is `Found(ids, false)` and therefore medium (libscope.rs:2189). Fixture p1: `$this->greet()` → `Greets::greet`, `$this->boot()` → `Base::boot`. Fixture p5: `Kid2::k2 -> Base::describe` medium, `static::mk()` → `Sq::mk` medium.
- **`parent::m()`** uses the first declared base of the caller's class (libscope.rs:2253-2260). Fixture p1 `parent::boot()` gives `Base::boot` high. Fixture p5 `parent::area()` gives `Sq::area` high.
- **Through a dependency's base** (`class Kid extends \Vendor\Pkg\Parent_`): `$this->inherited()` and `parent::boot()` → `External` (libscope.rs:2264-2276, 2190). Fixture p5 gives `<external>::this::inherited` and `<external>::parent::boot`. CHANGELOG.md:860-862 cites `$this->assertCount()` as PHPUnit's.
- **`$this->m()` with nothing found in-tree** → `Scoped::Ambiguous` (no binding, no name match). `static::m()` unknown → `None`, so later steps may still read it (libscope.rs:2276-2279).
- **`--dynamic-dispatch` / `inheritance_fanout`**: PHP is in `INHERITANCE_LANGS` (dispatch.rs:110-123). Fixture p4 with `--dynamic-dispatch` gives `Shape::area -> Sq::area / Ci::area / Base::area` and `Base::area -> Sq::area / Ci::area`, all low, `dispatch:inheritance`, `dynamic`. This also runs under `--dead-code`.
- **Typed parameter of an interface type** `function use_shape(Shape $s) { $s->area(); }` binds the interface declaration `Shape::area` high (fixture p4). The implementations are reached only through the dispatch fan-out.

Not supported or partial:

- **`trait_impl_target` / `dispatch::fanout`**: never set by php.rs, so interface → implementation fan-out comes only from `inheritance_fanout`. The `impls: true` signal (php.rs:25) is declared, not extracted.
- **`member_of_calling_class` / bare implicit-`this`**: not applicable, since PHP has no implicit `$this`. A bare `f()` never reaches a method (`bare_call_is_never_a_method` includes php, intra_file.rs:50-54; fixture p14 reports `only()` as `not-in-scope-for-bare-call`).
- **`overrides_below`, `super-base-out-of-graph`, `nearest_class_copies`**: Java/C#/Kotlin/Python paths, not PHP. PHP is not in `CLASS_RECORDS_ARE_EXACT` (dispatch.rs:130), and its `ClassDecl.base_types` is always empty (php.rs:113).
- **Bases of a class with no methods are lost**: bases are stored only on method `DefRecord`s, and `PhpIndex.bases` is filled from definitions (libscope.rs:1971-1985). An empty intermediate class `abstract class MyBase extends TestCase {}` therefore breaks the chain. Fixture p9: `Deep extends MyBase` (and `MyBase extends TestCase`), and `Deep::testDeep` is not a PHPUnit entry and is reported dead.
- **Trait conflict resolution** `use A, B { A::m insteadof B; B::m as n; }`: `insteadof`/`as` aliases are not read (php.rs:120-137 reads only `name`/`qualified_name` children).
- **`__call` / `__callStatic` / `__get` magic dispatch**: calls to undeclared methods are not routed to them.
- **Wrong-owner binding at high in intra-file**: `class A { function r() { $this->x(); } } class B { function x() {} }` binds `A::r -> B::x` *high intra-file* (fixture p10). The intra-file pass binds a unique same-file name before `PhpIndex` sees `this`.
- **Overloading / arity narrowing**: not applicable (PHP is not in `OVERLOAD_ARITY_LANGS`, type_hints.rs:1583).
- **Extension methods, mixins beyond traits**: not applicable.

## 07. Scoping & shadowing

- **`FileFacts::locals`**: not supported. `locals::spec_for` has no `"php"` arm (locals.rs:381-392), so `shadowed-by-local` never fires for PHP. PHP variables are `$`-prefixed and cannot collide with function names, but `$fn()` variable calls are not recognised as locals either (they become `<external>::$fn`).
- **Lexical narrowing of nested definitions**: PHP is not in `lexically_scoped` (intra_file.rs:119-135). This is correct for PHP, where a nested `function` is global once defined. Fixture p10 binds `outer -> nested` high, both directly and from inside a closure.
- **Positional shadowing**: not applicable.
- **Conditional definitions**: supported (php.rs:395-411, `DefRecord::conditional`). `PhpIndex::one_copy` prefers the single unconditional definition when the include chain does not decide, and that binding is never certain (libscope.rs:2089-2112; CHANGELOG.md:687-690). Fixture p10: `run2` (no require) gives `get_multi` (inc/cache.php, unconditional) at medium. Two unconditional copies (`dup` in two files) are refused: `candidates-in-other-files {candidates: 2}`.
- **Namespace scoping** by byte position (`php_ns_at`, libscope.rs:1908-1914) supports several semicolon-form namespaces in one file. Braced namespaces are broken (§04).
- **Owner disambiguation in a file**: `A::y()` and `$this->y()` with `A::y` and `B::y` in one file pick `A::y` (fixture p10). A unique name in another class is still bound (§06).

## 08. Macros, preprocessor & codegen

- Not applicable: PHP has no macros or preprocessor, and nothing is tagged `macro:<name>`.
- **`name_mentions`**: not produced by the PHP plugin (php.rs never writes it).
- **Generated code**: there is no detection of generated files (Doctrine proxies, Laravel IDE helpers, `vendor/composer/autoload_*.php`). `vendor/` handling depends on the walker's deny list and `.gitignore`, not on anything PHP-specific (unverified for PHP specifically).
- **`eval()`, `create_function()`**: not modelled (§15).

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: there is no PHP-specific coverage. Pass A covers asm↔C, the Python links are Python-only, and Pass B's restricted families allow `napi` → JS/TS, `pyo3` → Python and `c-abi` → the C family (ffi.rs:694-701). PHP's `FFI::cdef()`, Zend extensions (`PHP_FUNCTION`) and `ext-*` bindings are not recognised. A grep of ffi.rs for `php` finds nothing.
- **Descriptor links** (`descriptor::link_descriptors`) are language-agnostic: a proto `rpc` links to any method whose owner name contains the service name. Fixture p11: `hello.Greeter.SayHello -> GreeterService::SayHello` (`?desc`, i.e. not high).
- **Markup code-behind** (`markup.rs`): C# only. Not applicable to PHP templates (Blade, Twig, `.phtml` partials).

## 10. Frameworks & entry points

There are 17 rules with `language: "php"` in `crates/cgg-core/src/frameworks/rules.rs`. No multi-language rule covers PHP: `RuleSpec.language` is a single string, and the grep count of `language: "php"` is 17. Matching is in `crates/cgg-resolve/src/frameworks/mod.rs`. String targets are decoded by `decode_string_target`, which handles `"C@method"` and `"C::method"` (frameworks/mod.rs:1904, tests at 2854-2868).

| rule id | kind | detect | trigger (attrs / registrars / base_types+methods) | node | evidence |
| --- | --- | --- | --- | --- | --- |
| `laravel` | network | `Illuminate\Support\Facades\Route`, `Illuminate\Routing`, `Illuminate`; paths `routes/web.php`, `routes/api.php`, `routes/console.php` | registrars `get post put patch delete any match resource apiResource redirect view` with string targets; base_types `Controller Command Job` × methods `index show store update destroy create edit handle` | yes | rules.rs:1035-1069; test `shape_e_laravel_supports_both_the_string_and_the_array_form` (crates/cgg/tests/frameworks.rs:330); fixture p2 `Route::get('/users',[UserController::class,'index'])` and `'UserController@store'`; fixture p8 `PostController::show`, `Cmd::handle` |
| `symfony` | network | `Symfony\Component\Routing\Attribute\Route`, `…\Annotation\Route`, `Symfony\Bundle`, `Symfony\Component\HttpFoundation` | attributes `Route Get Post Put Delete Patch AsCommand AsMessageHandler`; base_types `AbstractController Command` × methods `execute __invoke` | yes | rules.rs:1070-1100; test `shape_a_php_symfony_attribute_is_captured` (frameworks.rs:110); fixture p2 `#[Route('/x')]`; fixture p8 `XCmd::execute` |
| `wordpress` | network | paths `wp-content/`, `functions.php`; detect_calls `add_action add_filter register_rest_route add_shortcode` | registrars `add_action add_filter register_rest_route add_shortcode register_activation_hook`, string targets | yes | rules.rs:1101-1131; fixture p2 `add_action('init','my_init')`; fixture p8 `[$this,'on_init']`, `array('Plug','stat_cb')`, `'Plug::sc'` |
| `codeigniter` | network | `CodeIgniter`; path `app/Config/Routes.php` | registrars `get post put patch delete add match`, string targets; base_types `BaseController Controller` (no methods) | yes | rules.rs:1132-1148 (not fixture-tested) |
| `slim` | network | `Slim` | none: a gap rule | yes | rules.rs:2861-2877, `gap:` at 2874 |
| `cakephp` | network | `Cake` | none: a gap rule | yes | rules.rs:2878-2894, gap at 2891 |
| `yii` | network | `yii`, `Yii`, `Yiisoft` | none: a gap rule (no `action*` prefix matcher) | yes | rules.rs:2895-2911, gap at 2908 |
| `drupal` | network | `Drupal` | none: a gap rule (hooks in `.module`/`.install`, `*.routing.yml`) | yes | rules.rs:2912-2928, gap at 2925 |
| `magento` | network | `Magento` | none: a gap rule (XML-bound `execute`, plugins) | yes | rules.rs:2929-2945, gap at 2942 |
| `aws-lambda` | network | `Bref` | base_types `Handler HttpHandler SqsHandler S3Handler SnsHandler EventBridgeHandler PsrRequestHandler DynamoDbHandler KinesisHandler` × methods `handle handleRequest handleSqs handleS3 handleSns` | yes | rules.rs:4714-4749; frameworks.rs:1180 `aws-lambda/php`; fixture p8 `H::handle` |
| `gcp-functions` | network | `Google\CloudFunctions` | registrars `http cloudEvent`, string targets | yes | rules.rs:4750-4768; frameworks.rs:1241 `gcp-functions/php`; fixture p8 `FunctionsFramework::http('hello','helloHttp')` |
| `phpunit` | test | `PHPUnit` | attributes `Test Before After BeforeClass AfterClass`; base_types `TestCase PHPUnit\Framework\TestCase KernelTestCase WebTestCase DuskTestCase` × methods `test* setUp tearDown setUpBeforeClass tearDownAfterClass assertPreConditions assertPostConditions` | yes (one harness node) | rules.rs:7026-7056; §11 |
| `symfony-messenger` | queue | `Symfony\Component\Messenger` | attribute `AsMessageHandler`; base_types `MessageHandlerInterface MessageSubscriberInterface BatchHandlerInterface BatchHandlerTrait HandlerDescriptor` × methods `__invoke handle getHandledMessages shouldFlush process` | yes | rules.rs:10558-10586; fixture p8: the class-level `#[AsMessageHandler]` is not matched (see gaps) |
| `laravel-lifecycle` | lifecycle | `Illuminate\Support\ServiceProvider`, `…\Providers`, `…\Migration`, `…\Seeder`, `…\FormRequest`, `Illuminate\Mail\Mailable`, `…\Notification`, `Illuminate\View\Component` | base_types `ServiceProvider AuthServiceProvider EventServiceProvider RouteServiceProvider BroadcastServiceProvider Migration Seeder FormRequest Mailable Notification Component` × methods `register boot booting booted provides map mapWebRoutes mapApiRoutes up down run rules authorize messages attributes prepareForValidation passedValidation withValidator failedValidation build envelope content attachments headers via viaQueues viaConnections toMail toDatabase toArray toBroadcast toSlack toVonage shouldSend databaseType render mount shouldRender data` | **no** (root mark only) | rules.rs:10587-10664; fixture p8 `Prov::register`/`boot` rooted, `Prov::other` dead |
| `laravel-events` | queue | `…EventServiceProvider`, `Illuminate\Foundation\Http\Kernel`, `…\Facades\Event`, `InteractsWithQueue`, `Dispatchable`, `ShouldQueue`, `ShouldBroadcast` | none: a gap rule (listeners, middleware, observers wired by class string) | no | rules.rs:10665-10689, gap at 10686 |
| `swoole` | network | `Swoole`, `OpenSwoole` | none: a gap rule (`$server->on('request', closure)`) | no | rules.rs:10690-10706, gap at 10703 |
| `roadrunner` | network | `Spiral\RoadRunner`, `Spiral\Goridge` | none: a gap rule (worker loop) | no | rules.rs:10707-10723, gap at 10720 |

- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for`, `self_module_markers_for`**: no PHP entries (rules.rs:11666-11850; each match lists only java/csharp/rust/solidity/js). A PHPUnit `#[DataProvider('cases')]` therefore does not root `cases` (fixture p9: `AttrOnly::cases` reported dead).
- **Registrar verbs** come from the PHP rules: the union of laravel, wordpress, codeigniter and gcp-functions verbs, gated per language (`ExtractCtx::for_language`, test `the_registrar_gate_is_scoped_to_one_language`, crates/cgg-lang/src/lib.rs:390).
- **Attribute keys**: an attribute is recorded as the whole `#[…]` text, with stacked groups split (attrs.rs:97-110; test `stacked_attribute_groups_are_separate_attributes`, php.rs:437).

Gaps found:

- **A fully-qualified attribute is not matched**: `#[\Symfony\Component\Routing\Attribute\Route('/fq')]` creates no entry (fixture p12). Last-segment matching splits on `::`/`.`, not `\`. This is unverified in code, but observed.
- **Comma-grouped attributes** `#[Deprecated, Route('/z')]` are one attribute keyed by the first name, so the `Route` is missed. `#[Route('/y'), Deprecated]` is matched (fixture p13).
- **Docblock annotations** `/** @Route("/old") */` (Symfony ≤5 / Doctrine annotations) are not read (fixture p12).
- **Class-level attributes** `#[Route('/api')]` on a class and `#[AsMessageHandler]` on a class are not recorded, because attributes are collected only for function/method `DefRecord`s (php.rs:241). Fixture p8 `MH::__invoke` stays dead, and the coverage table says "symfony-messenger … entries NOT enumerated".
- **A WordPress registrar whose first argument is not a string** (`register_activation_hook(__FILE__, 'activate_me')`): the handler string is taken as the route string and skipped (registrar.rs capture: the first string equal to the route is dropped when `arg_count > 1`). Fixture p8: `activate_me` reported dead, and an unresolved `__FILE__` `value-ref-no-enclosing` was recorded.
- **Laravel**: `Route::get('/closure', function () {...})` closures have no callable to enter (fixture p2). `Route::controller()`/`Route::group()` are not registrars.
- Slim, CakePHP, Yii, Drupal, Magento, Laravel events/middleware/observers, Swoole and RoadRunner are gap-only rules: they are detected and reported in "seen, no rules", but no entries are enumerated.

## 11. Test harness entries

- **`phpunit`** (kind `test`, rules.rs:7026-7056) mints ONE node, `<framework-entry>::test::phpunit`, with entry edges to each case (fixture p2: `-> UserTest::setUp`, `-> UserTest::testIndex`). A case is selected by:
  - **Base class + name pattern**: a class whose bases include `TestCase` (or the other listed bases) × `test*` (case-sensitive prefix pattern) and the `setUp` family. Fixture p9: `ExtTest::testA` is an entry.
  - **Attribute** `#[Test]` (`Before`, `After`, …). Fixture p9: `AttrOnly::checks` (`#[Test] #[DataProvider('cases')]`) is an entry.
  - **Name-only gating**: a `test*` method in a class with no TestCase base, in a file that is neither a test file nor importing PHPUnit, is not a case. Fixture p9 `src/Prod.php`: `Prod::testConnection` and `Prod::setUp` are reported dead.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** includes `"php"` (rules.rs:11806, also at 180 of the helper block). Every other member of a TestCase subclass is a test root mark. Fixture p2: `UserTest::unusedHelper` and `UserTest::it_works` are "LIVE ONLY IN TESTS … [TestEntry]" (`--why-live`). Fixture p9: `ExtTest::provider` is not reported.
- **Test-file classification**: a PHP file is a test file if its name ends with `Test.php` (testfile.rs:106), or by the shared directory rule (`test/`, `tests/`, … testfile.rs:35-42).
- **`--no-test-entries`** turns the harness into root marks with no node. Fixture p9 under that flag shows no `framework-entry` lines in the graph.
- Not supported: indirect TestCase inheritance through a method-less intermediate class (fixture p9 `Deep::testDeep` dead; §06); `#[DataProvider]`/`@dataProvider` targets (§10); `#[\PHPUnit\Framework\Attributes\Test]` written fully qualified is not matched as an attribute (it is rooted in fixture p2 only through the TestCase class-member rule); Pest (`test('x', fn…)` / `it(…)`), Codeception, Behat contexts (`@Given`) and PHPSpec have no rules (grep of rules.rs finds no `pest`, `codeception`, `behat` or `phpspec`); `@test` docblock annotations are not read.

## 12. Dead-code analysis

- **Signal coverage** (`--dead-code` table, fixture p2/p6): `php … class degraded, vis no, attrs yes, refs yes, disp yes`. The class is "degraded" because no PHP callable carries a visibility (caps.rs:160-170). `refs`/`disp` show "yes" because the plugin *declares* `value_refs`/`impls` (php.rs:21-29), not because reference edges or `trait_impl_target` exist.
- **Visibility**: never extracted (php.rs:239), so every finding carries `language-lacks-visibility`, which caps it at medium (deadcode.rs:324; evidence.rs:320). With the default `--dead-code-confidence high`, PHP reports nothing (fixture p2: every finding was in the `medium` band). `private` methods are not treated as file/class-local and `public` ones are not exported roots (fixture p2 `UserController::deadPrivate` is medium, like `unusedAction`).
- **Roots**: `toplevel:invocation`, where a call in file-level script code roots its callee (roots.rs:552-610). Fixture p6: `script_fn` in a shebang script and `render_item` in a `.phtml` template are live. Other roots are framework entries (§10), test roots (§11), and `UNIVERSAL_ENTRY` `main` (roots.rs:20).
- **Dunder lifecycle**: PHP is in `DUNDER_LANGS` (roots.rs:160-161), but `is_dunder` requires a trailing `__` (roots.rs:967-969), so PHP magic methods (`__construct`, `__destruct`, `__invoke`, `__call`, `__callStatic`, `__get`, `__toString`) are never rooted. PHP has no `LIFECYCLE` entry (roots.rs:96-158). Fixture p2 reports `Ctor::__construct`, `Ctor::__destruct`, `Factory::__call` and `Factory::__callStatic` unreferenced, and fixture p8 reports `Plug::__construct`. `CONVENTIONAL_METHODS` (roots.rs:162-170) holds no PHP names.
- **`implements-external-contract`**: never applies to PHP. It needs an `@Override`/`override` marker (deadcode/mod.rs:409, 496-505) or Python. PHP 8.3 `#[\Override]` is recorded as the text `#[\Override]` and does not match. Fixture p6: `Hooked::marked` (`#[\Override]`, extends `Vendor\Base`) and `Hooked::unmarked` are both plain medium findings.
- **Reflection doubt** (§15): `method_exists($this, 'reflected')` adds `name-collides-with-screened-site:reflection` (fixture p2 `Factory::reflected`).
- **Unresolved-name doubt**: `name-matches-unresolved-site` drops a finding to low (fixture p2 `Factory::anon`, called via untyped `$o->anon()`).
- **Inheritance fan-out under `--dead-code`**: overrides of in-tree declarations are reached (dispatch.rs:110-123).
- Not supported: visibility-scoped reasoning, a magic-method lifecycle, `#[\Override]`, `__invoke`-as-callable roots, autoload/class-string roots (`$container->get(Foo::class)`), and hooks named by string in config (Drupal, Magento, Laravel `$listen`).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/php.txt`**: 324 non-empty, non-comment names (functions such as `array_map`, `strlen`, `call_user_func`, and classes such as `ArrayIterator`). It is loaded at stdlib.rs:196 and 237. Fixture p1 with `--include-stdlib` gives `<stdlib>::array_map`, `<stdlib>::strlen` and `<stdlib>::call_user_func`.
- **Case sensitivity**: the lookup is case-sensitive, although PHP function names are not. Fixture p3: `STRLEN("a")` became `<external>::STRLEN`, not stdlib.
- **Leading backslash**: `\strlen('y')` became `<external>::\strlen` (fixture p5). The global-namespace prefix is not stripped before the stdlib lookup.
- **Stdlib method names on untyped receivers**: not applicable, since PHP never reaches step-5 fan-out (§14).
- **Other tables**: `NULL_TYPE_MEMBERS` has a PHP entry (stdlib.rs:135). There are no PHP entries in `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `TOP_TYPES` or `CALLBACK_TYPE_CONSTRUCTORS`, and there is no `rust_types`-style wrapper table. There are no implicit packages: the global namespace is the fallback in `PhpIndex` (libscope.rs:2228-2229).
- **`--include-stdlib` / `--include-external`** behave as in every language: they mint `<stdlib>::`/`<external>::` leaves (fixture p1).

## 14. Confidence & certainty

- **High** comes from:
  - intra-file binding of a unique name. Fixture p1: `Base::boot -> Base::init`, `self::make`. It also applies to an unrelated class's method (fixture p10 `A::r -> B::x`, a wrong edge at high).
  - a `PhpIndex` `Bound { certain: true }` result: one callable, one declaring file, and found on the class itself rather than inherited (libscope.rs:2059-2063, 2187-2189, 2286-2293; cross_file.rs:5508-5510, `module_certain = certain && cids.len() == 1`). Fixtures: typed parameter `$x->help()`, `Helper::stat()`, `use function` → `fmt`, `parent::boot()`, the include-chain copy `run -> get_multi`.
  - a construction receiver `(new Helper())->help()` (fixture p1 L39).
- **Medium** comes from:
  - inherited or trait members reached through `this`/`static` (fixture p5 `Kid2::k2 -> Base::describe`, `static::mk`; fixture p1 `static::make`).
  - the conditional-fallback choice (fixture p10 `run2 -> get_multi`).
  - several copies in one file (fixture p1 `top -> helper_fn` ×2).
- **Low**: framework entries, `dispatch:inheritance` fan-out, and `<external>`/`<stdlib>` exit nodes.
- **Mermaid `?`** marks arrows with no high site. Fixture p1: `N0 -->|?| N1` (`top -> helper_fn`), `N12 -->|?2x| N1`. Descriptor links show `?desc` (fixture p11).
- **No duck-typed fan-out at all**: step 5 requires the receiver hint to start with a lowercase letter (cross_file.rs:6853-6860), and PHP receivers start with `$`. Every untyped `$obj->m()` is therefore left unresolved as `candidates-in-other-files`, even with a single candidate (fixture p3 `$u->uniqueM()`; fixture p14). `--fanout-cap` therefore has no effect on PHP method calls. This avoids wrong edges but is a large recall gap.
- `weak_receiver_type`, `OwnerFacts` typed-receiver certainty and `one_declaring_ancestor` are not wired for PHP (no PHP arms; inherited `PhpIndex` hits are always uncertain).

## 15. Dynamic use & reflection

- **dynuse**: PHP's reflective callees are `call_user_func`, `call_user_func_array` and `method_exists` (dynuse.rs:51-54). Capture runs only when `ctx.deadcode_signals` is set (php.rs:53-55). Test: `php_call_user_func_names_a_function` (dynuse.rs:217). Fixture p2: `method_exists($this,'reflected')` screens `Factory::reflected` (`reflection`). This is doubt only: no edge is created.
- **Not captured**:
  - **Namespaced strings** `call_user_func('App\Misc\target_fn')` fail `is_identifier_shaped`, because `\` is not allowed (dynuse.rs:58-66). Fixture p2: `target_fn` is reported with no reflection risk.
  - **Array callables** `call_user_func([self::class,'create'])`: only string arguments are read (dynuse.rs:118-121).
  - **Not in the allowlist**: `is_callable`, `property_exists`, `ReflectionClass`/`ReflectionMethod`, `new $class`, `$class::method()`, `$obj->$name()`, `array_map('f', …)`, `usort(…, 'cmp')`, `Closure::fromCallable`, `forward_static_call`, `constant()`, and `$$var`.
  - **`eval()` and `create_function()`**: not modelled.
- **Module load** (`DYN_MODULE_LOADERS` = `require`, `importScripts`, `import_module`, facts.rs:165): PHP's `require`/`include` with dynamic paths is not a dyn-use. They are import records (§04), and an unresolvable path just fails to narrow.
- **Magic dispatch** (`__call`, `__callStatic`, `__get`): not modelled (§06).

## 16. Known gaps & limitations

From code and docs:

- `gap:` strings on 8 PHP rules: slim, cakephp, yii, drupal, magento, laravel-events, swoole, roadrunner (rules.rs:2874, 2891, 2908, 2925, 2942, 10686, 10703, 10720).
- CHANGELOG.md:705-708: "Not covered (WS14b): … PHP functions with two unconditional copies (WordPress `__`)". Confirmed in fixture p10 (`dup`).
- type_hints.rs:720-724 and 3268-3271: PHP records no `scope_byte`, so locals stay file-wide, and the scoped lookup "cost PHP 8,142 typed-receiver edges" when it was tried. type_hints.rs:1465-1470: the C++ parameter parser must not be used for PHP.
- README.md:498 still lists PHP as `require_once/include | — | —`. That is stale: namespaces, `use`, and parameter and construction typing exist.
- No `#[ignore]` PHP tests were found (`grep -rn ignore` over the PHP test names above shows none).

Found while cataloguing, each with a fixture:

1. **`new Foo()` is never a call**: there are no edges to `__construct` (p2, p14).
2. **Magic methods are never runtime roots**, because `is_dunder` needs `__x__` (p2, p8). `__construct` and the others are false-positive dead code.
3. **Untyped `$var->m()` is never resolved**: there is no duck fan-out (cross_file.rs:6853-6860; p3, p14). There is also no local-assignment, property, docblock or chained-return typing (p1, p3).
4. **Braced `namespace X { }` is mis-recorded**: the whole block text is taken as the namespace, and the record is dropped beyond 400 bytes (php.rs:250-262; p4, p5).
5. **Wrong-owner high edge**: `$this->x()` binds another class's unique `x` in the same file (p10).
6. **Closures, arrow functions and anonymous-class methods**: closures are not callables, and anonymous-class methods are mis-owned (p1, p2).
7. **A case-mismatched or `\`-qualified call that resolves also emits an `<external>` leaf** for the same site (p3 `$s->UNIQUEM()`; p5 `\A\use_base`). The cause is unverified, probably case/backslash-sensitive audit reconciliation.
8. **Stdlib lookup is case-sensitive and does not strip a leading `\`** (p3 `STRLEN`, p5 `\strlen`).
9. **Attributes**: fully-qualified, comma-grouped, class-level and docblock attributes are missed (p8, p12, p13).
10. **Bases of method-less classes are lost**, which breaks inheritance chains (p9).
11. **No visibility**, so all dead-code findings are capped at medium and nothing shows at the default `high` band (p2). `#[\Override]` is ignored (p6).
12. **Detection**: `.inc`, `.module`, `.install` and `.php5` are not detected, and there is no `<?php` content detection (p6, p7).
13. **Include chain**: basename-only matching, and no narrowing among copies in one file (libscope.rs:2004-2009, 2059-2063; p1).
14. **Registrar quirk**: a handler string after a non-string first argument is dropped (p8 `register_activation_hook`).
15. **Callables as values** (`'fn'`, `[$this,'m']`, `f(...)`) give no reference edges outside registrar verbs, and first-class callable syntax is recorded as a direct call (p1, p2).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.php`/`.phtml`/shebang only; no `.inc`/`.module`/content detection; depth skip works |
| 02 Callables extracted | partial | functions, methods, interface/trait/enum members, conditional defs; no closures/arrow fns; anon-class methods mis-owned |
| 03 Call-site forms | partial | bare/member/nullsafe/static/FQ calls; `new Foo()` not a call; variable/dynamic calls and callable values unmodelled |
| 04 Imports & module resolution | partial | namespaces, `use` (grouped, function, alias), require chain via PhpIndex; braced namespaces broken; no composer/PSR-4 |
| 05 Type inference & receiver typing | minimal | typed params (nullable/union/promoted) and `(new X)->m()` only; no locals, fields, docblocks or return-type chains |
| 06 Inheritance, dispatch & overloading | partial | parent/self/static/$this through bases and traits, external bases, inheritance fan-out; wrong-owner intra-file high edge; method-less classes drop bases |
| 07 Scoping & shadowing | partial | conditional defs and include-chain copy narrowing; no locals; nested functions correctly global |
| 08 Macros, preprocessor & codegen | n/a | PHP has no macros; no generated-code detection |
| 09 FFI & cross-language links | minimal | only the language-agnostic proto→host descriptor link; no PHP FFI/extension links |
| 10 Frameworks & entry points | partial | 17 rules (9 enumerate, 8 gap-only); FQ, comma-grouped, class-level and docblock attributes missed |
| 11 Test harness entries | partial | PHPUnit base+`test*` and `#[Test]`, class members as test roots; no DataProvider, Pest, Behat, indirect bases |
| 12 Dead-code analysis | minimal | no visibility so everything is capped at medium; magic methods never rooted; `#[\Override]` ignored |
| 13 Builtins & stdlib tables | partial | 324-name php.txt; case-sensitive and `\`-prefixed lookups miss |
| 14 Confidence & certainty | partial | PhpIndex certainty is sound for own-class lookups; no duck fan-out; one wrong high (intra-file owner) |
| 15 Dynamic use & reflection | minimal | `call_user_func`/`call_user_func_array`/`method_exists` with plain-identifier strings only |
| 16 Known gaps & limitations | n/a | 15 fixture-backed gaps listed above |
