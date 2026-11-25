# objc

> Revised 2026-11-25 against cgg 0.13.3: the plugin now records plain C calls, `[[T alloc] init…]` constructions, `@selector`/`NSSelectorFromString` and C function values as references, tags `@protocol` members `abstract`, and marks preprocessor branches (same-file alternatives bind at `medium`); declared return types type `[[F make] work]`. Sections 01–16 are corrected accordingly. `file:line` citations not otherwise revised date from the 0.11.0 audit and may have drifted.

The Objective-C plugin (`ObjcPlugin`) is in `crates/cgg-lang/src/plugins/objc.rs`. It is 545 lines, including inline tests. It parses `.m` and `.mm` with the crates.io grammar `tree-sitter-objc` (workspace pin `"3.0"`, Cargo.toml:104; locked at 3.0.2, Cargo.lock:1644-1645), loaded through `tree_sitter_objc::LANGUAGE` (objc.rs:18-20). Nothing is vendored.

One recursive `ObjcWalker` (objc.rs:71-230) handles these node kinds:

- `class_implementation` / `class_interface`: pushes the class name as scope.
- `method_definition` / `method_declaration`: records a method (`abstract` inside a `@protocol`).
- `function_definition`: records a C free function, named through pointer layers by `c::fn_name_from_declarator`.
- `call_expression`: records a plain C call through `c::call_ref` (a call through a field is skipped), and `NSSelectorFromString(@"…")` as a selector value.
- `message_expression`: records a call site; `[[T alloc] init…]` becomes a typed call on `T`.
- `selector_expression` (only when reference sites are on): `@selector(…)` as a selector value.
- `preproc_include` / `preproc_import`: records an import.

Everything else is only descended into. After the walk, `c_values::collect` (reference sites only) records C function values, and `preproc::mark` records each definition's preprocessor branches (objc.rs:37-47). There is no `resolver_kind` override, no `signals()` override (so every dead-code signal is `false`), no `shebangs`, no `ts_language_for` and no `max_line_bytes`.

Obj-C belongs to the C family (`language_family`, cgg-core/src/lang_family.rs:28: `"c" | "cpp" | "objc"`). That gives it:

- prototype unification (cgg/src/lib.rs:508-535);
- `#include`/`#import` closure following (cross_file.rs:1789, 1816);
- the qualified-name sibling-language retry (cross_file.rs:7363);
- the asm↔C FFI pass (ffi.rs:66, 127).

It is not part of the C/C++ function-macro expansion (cgg/src/lib.rs:595 filters to `"c" | "cpp"`; re-checked 2026-11-25).

Fixtures cited below are under `<scratch>/parity/objc/` (f1 … f6). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, and dumped with `show.py` (graph) and `aud.py` (per-file audit buckets) in the same directory. `--dead-code` runs used `--no-graph --dead-code-confidence low`.

## 01. Detection & parsing

- **Extensions `.m` and `.mm`** (objc.rs:15-17). Fixture f1: `Service.m` → `detected_via: "extension:.m"`; fixture f3: `mix.mm` → `extension:.mm`.
- **Obj-C headers are not claimed.**
  - A `.h` is always C or C++ by the header verdict (detect.rs `header_verdict`, see c.md §01), and the sniff has no Obj-C marker.
  - Fixture f1: `Service.h` holding `@interface Service : NSObject … @end` → `language: c`, `header-heuristic:c`, `parse_status: error`, 0 callables. Its `@property` line even produced a spurious C call site `property` (external bucket).
  - Fixture f4: `P.h` (`@interface P … @end`) → `c`, `error`.
  - Consequences:
    - `@interface`/`@protocol` declarations in headers are never Obj-C facts.
    - A `#import <UIKit/UIKit.h>` that lives only in a header is never seen by the Obj-C framework detector (§10).
- **`.mm` (Objective-C++) is parsed with the Obj-C grammar, not C++.** C++ constructs mis-parse. Fixture f3 `mix.mm` → `parse_status: error`:
  - `namespace ns { int cppfn(…) }` yields `cppfn` with no `ns::` scope;
  - `class K { void m() {} }` yields nothing;
  - `k.m()` and `ns::cppfn(1)` yield no call sites.
- **Shebangs**: none (trait default, lib.rs:300-302).
- **Content detection**: none. `.m` is also the MATLAB/Mercury extension. No other plugin registers it (`grep '".m"' crates/cgg-lang/src/plugins/*.rs` finds only objc.rs), so every `.m` is parsed as Obj-C.
- **Grammar**: one grammar, no dialect switch, no `ts_language_for`.
- **Re-parse/fallback**: none. A file with errors is still extracted with `parse_status: "error"`. Fixture f6 `mac.m` (file-scope `MKFN(one)` without `;`) → `error`, and the next function `g` is recorded with its start line pulled up to the macro line (start_line 2 for a definition on line 3).
- **Depth skip** (shared): a tree deeper than `MAX_TREE_DEPTH` (4,000, cgg/src/lib.rs:155) is skipped. Fixture f6 `deep.m` (5,000 nested parentheses) → `file_skipped`, `reason: too-deep, detail 4000`.
- **Minified / long-line handling**: no `max_line_bytes`. No locals table is recorded (§07). Not exercised (unverified).

## 02. Callables extracted

Supported:

- **Instance method in `@implementation`**: `- (void)run { … }` → `DefVariant::InherentMethod`, qn `Service::run`, `has_body: true` (objc.rs:174-215, has_body at 212). Test `class_methods` (objc.rs:239). Fixture f1.
- **Class method**: `+ (instancetype)create { … }` → `StaticMethod` (objc.rs:185-192, `+` detected by text prefix). Test `class_methods`. Fixture f1 `Service::create`.
- **`init` constructor**: a method whose name is exactly `init` → `DefVariant::Constructor` (objc.rs:187-188). Fixture f1 `Service::init` kind `constructor`.
  - `initWithName:` is **not** a constructor: it is an ordinary `method` (fixture f1 `Service::initWithName`).
  - Neither `new`, `alloc` nor `+initialize` is special.
- **Category implementation**: `@implementation Service (Extra) - (void)extraThing … @end` → `Service::extraThing`. `child(1)` is the class identifier, so the category name is dropped (objc.rs:61-66). Fixture f1.
- **Method declarations** (`@interface` in a `.m`, class extension, category interface) → recorded with `has_body: false` (objc.rs:210-212), then absorbed by `unify_declarations` when a body exists (cpp.rs:1335, invoked from lib.rs:534). Fixture f1: `extraThing` declared in `@interface Service (Extra)` and defined → one node. Fixture f2: `@interface A` declarations of `ping`/`setX` → only the bodies survive.
- **C free functions in `.m`**: `static int helper(int x) { … }` and `int main(…)` → `FreeFunction`, `qualified_name == simple_name`, `has_body: true` (objc.rs:81-108).
  - The name is read through pointer, attribute and parenthesis layers as the C plugin reads it (`c::fn_name_from_declarator`; WS22), and a declarator with no function declarator in it is no function (error recovery's `if (x) { … }`).
  - Fixture f1 `helper`, `c_func`, `main`.
- **Protocol methods**: `@protocol Runner - (void)go; @optional - (void)maybe; @end` → `method` nodes named `go` and `maybe` with **no owner** (`protocol_declaration` does not push scope; only `class_implementation`/`class_interface` do, objc.rs:61). They keep `has_body: false`, are tagged `abstract` (WS22), and survive as nodes because no same-qn body exists. Fixture f1 (`go`, `maybe`), fixture f5 (`greet`); `lang-docs/objc/f1` (2026-11-25): `go ['abstract']`.
- **Qualified names**: `Class::selectorHead`, joined with `::` (objc.rs:51-57). They carry no file, module or category.

Partial / wrong:

- **Multi-keyword selectors are truncated to their first keyword.** `record_method` takes the first `identifier`/`keyword_selector` child (objc.rs:176-180).
  - `- (int)addA:(int)a toB:(int)b` → `Service::addA` (fixture f1).
  - `- (void)setX:y:` and `- (void)setX:z:` are two nodes both named `A::setX` (fixture f2). Both call sites (`[a setX:1 y:2]`, `[a setX:1 z:2]`) bound to the **second** one (`A::setX@13`), so `setX:y:` had no incoming edge.
  - `use:other:proto:` → `D::use` (fixture f2).
  - `tableView:didSelectRowAtIndexPath:` → `AppVC::tableView` (fixture f3).
- **`+` and `-` methods with the same selector share a qualified name.** `+ (void)make` and `- (void)make` → two `G::make` nodes; `[G make]` is then `ambiguous-in-file` and unresolved (fixture f5).
- **Owner-less protocol members collide by name** with every same-named method (§06).

Not supported:

- **`@property`**: no getter/setter nodes (fixtures f1, f2 `@property … val`). An explicit getter `- (int)val` is a normal method.
- **`@synthesize` / `@dynamic`**: no nodes (fixture f6 `syn.m`: only the explicit `D::val`).
- **Blocks** (`^(int v){ … }`): not callables. Message sends inside a block are attributed to the enclosing method. Fixture f1: `[self work]` inside a `dispatch_async` block → edge from `Worker::work`.
- **Function-like `#define`**: not recorded as callables (no `preproc_function_def` arm). Fixture f2 `CALL_PING`, `MKFN` absent.
- **Macro-generated functions**: not expanded (§08).
- **C++ classes, members and namespaces in `.mm`**: not extracted (fixture f3).
- **`@interface` in `.h`**: parsed as C, so nothing is extracted (§01).
- **Visibility**: always empty. `static` C functions, `@private` ivars and class-extension-only methods are indistinguishable (objc.rs:101, 208).
- **Attributes**: always empty, so `IBAction`, `__attribute__((constructor))`, `NS_DESIGNATED_INITIALIZER` and similar are not recorded (objc.rs:102, 209). Fixture f3 `AppVC::tapped` (an `IBAction`) has `attributes: ''`.

## 03. Call-site forms

Supported:

- **Message send to `self`**: `[self helper2]` → `RefRecord { name: "helper2", receiver_hint: "self" }` (objc.rs:110-130). Test `message_sends` (objc.rs:254). Fixture f1: `Service::run → Service::helper2`, high, intra-file.
- **Message send to a class**: `[Helper create]` → receiver `Helper`. Test `message_sends`. Fixture f1: `[Service create]` → `Service::create`, high, `cross-file:imports`.
- **Message send to a variable**: `[s run]` → receiver `s`, bound by name (fan-out) unless typed (§05). Fixture f1 `[s run]` → `Service::run` medium.
- **Constructions (WS21)**: in `[[T alloc] init…]` the outer send's receiver is rewritten to `T` and marked `typed_receiver` (`alloc` is objc's entry in `CONSTRUCTING_METHODS`). Fixture `lang-docs/objc/f1` (2026-11-25): `[[Engine alloc] initWithName:@"a"]` → `Engine::initWithName`, high.
- **Nested / chained sends**: `[[Engine shared] start]` records both sites; the inner one types the outer receiver from its declared return type (`+ (Engine *)shared`, WS23, objc is in `TYPE_FIRST_RETURN_LANGS`). Fixture `lang-docs/objc/f1`: `[[Engine shared] start]` → `Engine::start` high, not `Motor::start`.
- **`super` sends**: `[super run]` → receiver `super`.
  - With no in-graph candidate → `super-base-out-of-graph` (cross_file.rs:2844-2856). Fixture f1 `[super init]`; fixture f3 `[super viewDidLoad]`.
  - With candidates, it is by-name fan-out, not a base-chain walk (fixture f2 `[super ping]` in `C : A` → `A::ping` **and** `B::ping`, both medium; re-checked 2026-11-25, `lang-docs/objc/f2`).
- **Multi-keyword sends** are recorded under the `method` field: the first keyword only (`[a setX:1 y:2]` → `setX`; fixture f2). This matches the truncated definition names, so they line up, but different selectors that share a first keyword collide.
- **Plain C calls (WS22)**: `helper(x)`, `NSLog(…)` are recorded as the C plugin records them (`c::call_ref`); a call through a field (`self.block(x)`, `s->fp(x)`) is not recorded. Fixture `lang-docs/objc/f1` (2026-11-25): `helper(3)` → `helper` high; `NSLog` goes to the stdlib bucket. Fixture `lang-docs/objc/f2`: `c_side(1)` in a method binds `c_side` in `util.c` through `#import "util.h"`, high.
- **Selectors and function values (WS26)**: under `--reference-edges` or `--dead-code`, `@selector(onNote:)` and `NSSelectorFromString(@"onNote:")` with a literal are value references named by the first keyword, receiver `cgg_core::OBJC_SELECTOR_RECEIVER`; they bind the method of that selector on every class declaring one (`high` for one, `medium` for several, refused over `--fanout-cap`). C function values in `.m`/`.mm` are read by `c_values` (ObjC methods are frames, their parameters bindings); a bare value is kept only when it names a C function, never a method, and a name the source continues with `::` or `(` is never a value. Fixture `lang-docs/objc/f1`: `selector:@selector(onNote:)` → `Service::onNote` and `CFDictionaryApplyFunction(nil, applier, nil)` → `applier`, both `via: reference`, high.
- **Message sends in blocks** are recorded and attributed to the enclosing method (fixture f1 line 17).
- **Fan-out cap**: by-name candidates are bounded by `--fanout-cap` (generic, cross_file.rs `FanoutCapExceeded`). Not exercised over the cap for objc (unverified).

Not supported:

- **Property / dot syntax**: `self.val`, `s.name = …`, `self.delegate` are not calls (fixture f6 `prop.m`: `E::a` has no site; fixture f2 `self.val` → no edge, while `[self val]` → `D::val` high). A dot-syntax receiver inside a send survives as text: `[self.delegate greet]` → receiver `self.delegate` (fixture f5).
- **`performSelector:` with a computed selector**: nothing (only literal `@selector`/`NSSelectorFromString` forms are read).
- **Registrar arguments**: not recorded; the plugin never calls the shared `registrar.rs` capture. `value_refs` is still not a declared signal (observed instead).
- **Arity**: `RefRecord::arity` is never set; objc is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:2357).
- **Explicit generics** (`NSArray<NSString *> *`): not applicable to sends; nothing recorded.
- **Casts as receivers**: `[(A *)o ping]` → receiver text `(A *)o`. It is not peeled (the walker does not call `receiver::peel`). The site went to unresolved (`candidates-in-other-files`, 3 candidates) and also to the external bucket (fixture f2).
- **C++ calls in `.mm`**: `k.m()`, `ns::cppfn(1)` → nothing (fixture f3).
- **Duplicate unresolved records**: `[self run]` inside a block in a method of a subclass produced two unresolved records at the same line (`no-candidate-in-file` and `candidates-in-other-files`) (fixture f1 `main.m:13`). Cause not investigated (unverified).

## 04. Imports & module resolution

Supported:

- **Quoted `#import "Foo.h"` / `#include "Foo.h"`** → `ImportRecord { kind: "include", path: "Foo.h" }` (objc.rs:131-156). Test `import_directive_captured` (objc.rs:270).
  - It is followed through the C-family include closure, depth 8 (`"include" if same_family(lang,"c")`, cross_file.rs:1789).
  - Fixture f4: `util.c` (C) → `objc_side` defined in `helper.m`, high, `cross-file:imports`, via `helper.h`.
- **Angle-bracket `#import <UIKit/UIKit.h>`** → `kind: "system-include"`, path kept **with** the `<…>` brackets (`raw` is only stripped of `"`, objc.rs:139).
  - It is followed only when not a platform header (cross_file.rs:1815-1827, `matches!(lang, "c"|"cpp"|"objc") && !is_platform_header`).
  - Framework detection matches the literal `<UIKit` prefix (§10). Test `detect_prefixes.rs:52-57` builds the objc probe as `#import <X/X.h>`.
- **`@import UIKit;` (module import)**: no arm for it (objc.rs only handles `preproc_include`/`preproc_import`). Unverified whether the grammar produces a distinct node; nothing is recorded.
- **Prototype unification across the family**: a `.h` prototype (parsed as C) is dropped when a `.m` body exists. CHANGELOG.md:1954-1959 ("Prototype unification covers C and Objective-C bodies"). Fixture f4: the `helper.h` prototypes `objc_side`/`c_side` do not appear as nodes.
- **Family qualified-name retry**: a qn lookup that misses in objc retries `c` and `cpp` (cross_file.rs:7363-7371; CHANGELOG.md:1894-1900).
- **`c_include_copies`**: C-family name narrowing to the caller's include closure applies (cross_file.rs:5712-5730, `same_family(lang,"c")`).

How cross-file binding actually happens:

- Message sends bind cross-file by owner/name, labelled `cross-file:imports`, whether or not anything is imported.
  - Fixture f3: `App.m` imports only `Api.h`, which is a C-parsed header with no Obj-C facts.
  - There, `[Engine shared]` → `Engine::shared` in `EngineImpl.m` was high.
  - `[eng start]` fanned out to `Engine::start` and to `Motor::start` in `Other.m`, a file nothing imports (both medium).
- The include closure does not scope message sends.

Not supported:

- **External-module refusal (`heads.rs`)**:
  - objc feeds `WorkspaceVocab::cpp_scopes` from field types, type aliases and qn segments (heads.rs:168-175, 194-215).
  - `FileHeads::external_head` returns `None` for objc (heads.rs:654-674: only python/js/ts/go/java/kotlin/scala/csharp/cpp).
  - So a send to a framework class is never refused as `external-module`. It falls into the external/stdlib *name screen* instead (§13).
- **`not-in-imported-module`**: not applicable (no `ModuleIndex` path for objc).
- **`FileFacts::packages`**: never set. There are no manifest reads (no Podfile, Package.swift or xcodeproj).
- **`@class Foo;` forward declarations, `@compatibility_alias`**: not recorded.

## 05. Type inference & receiver typing

The plugin records **no** `local_types`, `field_types`, `type_aliases` or `scoped_types`; the only `typed_receiver` flag it sets is for `[[T alloc] init…]` constructions (§03).

Supported (generic, weak):

- **Lowercase-name heuristic** (`find_constructor_assignments`, type_hints.rs:2404-2460, applied as Strategy 2 at 863-867):
  - A receiver variable whose name is the lower-cased form of a class declared **in the same file** is retyped to that class.
  - Fixture f2: `A *a …; [a ping]` → `A::ping` high, intra-file; `B *b; [b ping]` → `B::ping` high.
  - The declared type is irrelevant; only the variable name matters. In fixture f3, `Engine *eng` and `Engine *engine` (class in another file) both fanned out to `Engine::start` + `Motor::start`.
  - The rewrite is marked weak (`weak.insert(i)`).
- **`self` receiver**: binds to the enclosing class's member in-file at high (fixture f2 `[self ping]` in `C::doit` → `C::ping` high; fixture f1 `[self helper2]`).
- **Class-name receiver**: `[Service create]` binds the owner's method at high when the owner name is unique (§14).
- **Declared return types of a send used as a receiver**: `+ (Engine *)shared` types `[[Engine shared] start]` (§03, WS23).

Not supported:

- Parameter types (`- (void)use:(A *)param` → `[param ping]` fanned out to `A::ping`, `B::ping`, `C::ping`, medium, fixture f2).
- Declared local types (`Engine *eng = …`, fixture f3).
- Return types for a local (`Engine *eng = [Engine shared]; [eng start]` still fans out to `Engine::start` and `Motor::start` at medium, `lang-docs/objc/f1`); only a send used directly as a receiver is typed.
- Casts (`(A *)o`, fixture f2).
- `id<Proto>` protocol-qualified ids.
- `instancetype`.
- Property and ivar types (`self.delegate`, `_name`).
- Block parameter types (`^(id obj, …){ [obj ping]; }` → fan-out, fixture f2).
- No `declared_type` / nullability handling (`nullable`, `_Nonnull`, `__kindof`) — no objc entries in `MAYBE_TYPE_WRAPPERS`/`NULL_TYPE_MEMBERS` (stdlib.rs: objc appears only at lines 197, 238).
- No implicit receivers, no `weak_receiver_type`, no `module_typed_locals`.

## 06. Inheritance, dispatch & overloading

- **Base classes are not recorded.** `@interface C : A` produces no `ClassDecl` and no base record (objc.rs has no base handling). `bare_call_reaches` documents this: "Swift, Objective-C, whose plugins record no base types, so an inherited member cannot be told from an unrelated one" (cross_file.rs:4086-4088).
- **Inherited `self` sends fail.** `[self run]` inside `Worker : Service`, where `run` is defined only in `Service` in another file → unresolved `candidates-in-other-files` (fixture f1 `main.m:13`). `resolve_via_bases` has nothing to walk.
- **`super` sends**: by-name fan-out to every same-named method, or `super-base-out-of-graph` when there is none (§03). Fixture f2: `[super ping]` in `C::ping` → `A::ping` + `B::ping` (B is unrelated).
- **Protocol conformance** (`@interface G : NSObject <Greeter>`): not recorded.
  - There is no `trait_impl_target`, so `--dynamic-dispatch` adds no edges. Fixture f5 with `--dynamic-dispatch` shows the same edges as without.
  - Protocol method declarations are owner-less nodes tagged `abstract` (WS22), so the "drop abstract when a concrete candidate survives" narrowing removes them from a fan-out that also has a concrete `greet` (0.11.0 fixture f5 bound both; not re-run).
- **`inheritance_fanout`**: objc is not in `INHERITANCE_LANGS` (dispatch.rs:110+).
- **`member_of_calling_class` / `overrides_below` / `nearest_class_copies`**: Java/C#/Ruby-only paths; objc is not included (cross_file.rs:4075-4095 doc).
- **Categories / class extensions**: methods are merged into the class qn (`Service::extraThing`). That is correct ownership, but the category name is lost (fixture f1).
- **Overloading**: Obj-C has no overloading by type. Selectors are the identity, and they are truncated, so distinct selectors with a shared first keyword collide and cannot be told apart (§02, fixture f2 `setX`). Not in `OVERLOAD_ARITY_LANGS`.
  - `one_method` (cross_file.rs:4790+) would treat several same-qn candidates in one file as one overload set only for overloading languages. Observed: both `setX` sites bound to one node, not both.
- **Class vs instance method of the same selector**: same qn, so `ambiguous-in-file` (fixture f5).
- **`unique_owner_names`**: objc is in the "qualified name carries the module" group (cross_file.rs:4411). In practice it does not, since objc qns are `Class::sel` with no module; two classes of one name in different directories count as one site.

## 07. Scoping & shadowing

- **`FileFacts::locals`**: not populated. `locals.rs` has no objc table (`locals::collect` reads the C walk only for `c` and `cpp`, locals.rs:524). So `shadowed-by-local` never applies to call sites. The value-reference filter in `c_values.rs` does skip names a parameter or declaration of the enclosing function or ObjC method binds, and a field a C or C++ header declares is data in ObjC files too.
- **Lexical narrowing of nested definitions**: not applicable (no nested callables); objc is not in the narrowing list (CLAUDE.md cgg-resolve §3).
- **Positional shadowing**: none.
- **Conditional definitions** (`#if`/`#ifdef` alternatives): `preproc::mark` records `DefRecord::preproc_branches`, and objc is in `PREPROC_ALTERNATIVE_LANGS` with C (intra_file.rs:1025): same-named C functions that each sit in a different branch of one chain bind all at `medium` (WS36). Fixture `lang-docs/objc/f1` (2026-11-25): `plat()` under `#ifdef WIN … #else …` binds both `plat`s, medium.
- **Scope of C functions**: a `function_definition` inside `@implementation … @end` is recorded with the bare name, not the class scope (objc.rs:92-94). Correct for C semantics.

## 08. Macros, preprocessor & codegen

- **Function-like `#define`**: not recorded as a callable or as an `FnMacro` (no arm in objc.rs). Fixture f2 `CALL_PING`, `MKFN` → no node.
- **Function-defining macro expansion** (`c_macro::expand`): explicitly limited to `"c" | "cpp"` (cgg/src/lib.rs:527-531). Fixture f6 `mac.m`: `MKFN(one)` does not generate `fn_one`, and the file parses with an error.
- **`toplevel:macro-invocation` dead-code rule**: generic over `same_family(.., "c")` (roots.rs:616-660). No objc callable carries attribute `macro`, so it never fires for objc.
- **`#if` / `#ifdef`**: no blanking or re-parse. Branches are parsed as the grammar sees them; same-file alternatives are reconciled by `preproc_branches` (§07).
- **`name_mentions`**: never populated.
- **Generated code** (`macro:<name>` tags), templates, quote/unquote: not applicable / none.
- **`NS_ENUM`, `NS_OPTIONS`, `NS_ASSUME_NONNULL_BEGIN`, `@synthesize`**: not interpreted.

## 09. FFI & cross-language links

- **asm → objc C function**: supported through the C-family asm pass (ffi.rs:36-186, `c_family_lookup` uses `language_family("c")`, ffi.rs:66). Fixture f4: `boot.s` `call objc_side` → `objc_side` in `helper.m`, medium, `Via::Ffi("asm-c")`, `ffi-linker`.
- **objc → asm**: the C → asm direction runs for any C-family file and iterates `f.references`, which now include plain C calls (§03), so a C call to an asm label from `.m` should link (not re-run with a fixture).
- **C/C++ → objc C functions**: supported through include closure plus family retry (fixture f4: `c_side` (C) → `objc_side` (objc), high).
- **objc → C/C++ functions**: supported since WS22 (C calls recorded). Fixture `lang-docs/objc/f2` (2026-11-25): `c_side(1)` in `A::ping` → `c_side@util.c`, high, `cross-file:imports`.
- **`c-abi` exports (Rust `#[no_mangle] extern "C"`)**: the allowed source languages include objc (`language_family("c")`), and objc now records C calls, so the Pass-B prototype link can apply (0.11.0 fixture f4 predates this; not re-run).
- **Swift ↔ Obj-C bridging** (`@objc`, bridging headers, `NS_SWIFT_NAME`): not modelled. Swift's plugin and objc are not in one family (lang_family.rs:28).
- **JNI / P/Invoke / PyO3 / napi / wasm-bindgen**: not applicable.
- **Descriptor links** (protobuf rpc → host): no objc host mapping (descriptor.rs `family_of`; not verified for objc, none found by grep).
- **Markup code-behind** (`.xib`, `.storyboard`): not read. `markup.rs` has no storyboard/xib support (grep finds none). `IBAction`/`IBOutlet` wiring is invisible (named in the `objc-selector` gap text, rules.rs:5398).

## 10. Frameworks & entry points

Framework detection is per exact language (frameworks/mod.rs:241, rules keyed by `rule.language`). It is triggered by an objc file's import whose path has a rule prefix as a cut (mod.rs:264-272). Because angle-bracket paths keep their `<`, both `<UIKit` and the bare `UIKit` forms are listed. Matching of `methods` is by simple name (the first selector keyword), case-insensitive for exact names, with case-sensitive `prefix*` patterns (`name_pattern_matches`, mod.rs:570-584).

Every rule whose `language` is `objc`:

| rule id | kind | trigger (detect) | matcher | node | evidence |
|---|---|---|---|---|---|
| `uikit-lifecycle` | Lifecycle | import prefix `<UIKit`/`UIKit`/`<AppKit`/`AppKit`/`<Cocoa`/`Cocoa`/`<WatchKit`/`WatchKit` | methods: `viewDidLoad`, `loadView`, `viewWillAppear`, `viewDidAppear`, `viewWillDisappear`, `viewDidDisappear`, `viewWillLayoutSubviews`, `viewDidLayoutSubviews`, `viewDidLayout`, `viewWillTransitionToSize`, `updateViewConstraints`, `didReceiveMemoryWarning`, `viewDidUnload`, `awakeFromNib`, `prepareForSegue`, `layoutSubviews`, `drawRect`, `didMoveToSuperview`, `willMoveToSuperview`, `traitCollectionDidChange`, `touchesBegan`, `touchesMoved`, `touchesEnded`, `touchesCancelled`, `prepareForReuse`, `willActivate`, `didDeactivate`, `awakeWithContext`, `dealloc` | false (root mark only) | rules.rs:5196-5250; fixture f3: `AppVC::viewDidLoad` root `uikit-lifecycle:base-type` |
| `uikit-appdelegate` | Lifecycle | same import prefixes as above | methods: `application`, `applicationDidFinishLaunching`, `applicationWillFinishLaunching`, `applicationDidBecomeActive`, `applicationWillResignActive`, `applicationDidEnterBackground`, `applicationWillEnterForeground`, `applicationWillTerminate`, `applicationShouldTerminate`, `applicationShouldTerminateAfterLastWindowClosed`, `applicationShouldHandleReopen`, `applicationSupportsSecureRestorableState`, `applicationDidReceiveMemoryWarning` | false | rules.rs:5252-5290; fixture f3: `AppDelegate::application` (`application:didFinishLaunchingWithOptions:`) rooted |
| `cocoa-delegate` | Lifecycle | `<Foundation`/`Foundation`/UIKit/AppKit/Cocoa/`<WebKit`/`WebKit` | methods: `URLSession`, `tableView`, `collectionView`, `numberOfSectionsInTableView`, `numberOfSections`, `scrollViewDidScroll`, `textFieldShouldReturn`, `textViewDidChange`, `pickerView`, `searchBar`, `webView`, `splitViewController`, `navigationController`, `tabBarController`, `gestureRecognizer`, `imagePickerController`, `locationManager`, `centralManager`, `observeValueForKeyPath`, `encodeWithCoder`, `initWithCoder`, `copyWithZone`, `supportsSecureCoding` | false | rules.rs:5292-5342; fixture f3: `AppVC::tableView` rooted; fixture f2 (Foundation only, no match) → reported "detected, but no entry point matched its rules" |
| `objc-main` | Cli | `<UIKit`/`UIKit`/`<Foundation`/`Foundation`/`<Cocoa`/`Cocoa` | none (gap rule) | false | rules.rs:5344-5366; gap text printed in `seen_no_rules` (fixtures f2, f3). `main` is rooted anyway by the universal `builtin:main` (roots.rs:19) |
| `xctest` | Test | `<XCTest`/`XCTest` | methods `test*`, `setUp`, `tearDown` | true (one harness node `<framework-entry>::test::xctest`) | rules.rs:5368-5384; fixture f3: `MyTests::setUp`, `MyTests::testSomething` get `entry` edges (low); `testingNot` not matched (pattern boundary, mod.rs:574-578) |
| `objc-selector` | Lifecycle | `<Foundation`/`Foundation`/`<UIKit`/`UIKit` | none (gap rule) | false | rules.rs:5385-5401; disclosure only |

- **Rules listing several languages**: none include objc. Each `RuleSpec` has a single `language` field, and `grep objc rules.rs` hits only the six rows above. The four Swift namesakes (`uikit-lifecycle` 4978, `uikit-appdelegate` 5022, `cocoa-delegate` 5059, `xctest` 5123) are `language: "swift"` and do not apply to `.m`.
- **Stale gap text**: `xctest`'s `gap` string says cgg "has no prefix matcher", but the rule has `methods: ["test*", …]` and works (fixture f3). Since `has_matchers()` is true, the gap is not printed (mod.rs:395-404).
- **Detection blind spot**: only imports recorded by an objc file count. A UIKit/Foundation import that appears only in a `.h` (parsed as C) or a prefix header (`.pch`, not claimed by any plugin) does not activate any objc rule. Fixture f1: `Foundation` imported only in `Service.h` → `recognised: []`, `seen_no_rules: []`.
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for`**: no objc entries (rules.rs contains no other objc lines).
- **Registrar verbs**: none (the objc plugin captures no registrar arguments; selectors passed to `addTarget:action:` and similar are read as selector values instead, §03).
- **Base types**: every objc rule has `base_types: NONE`. Matching is name-only, gated by the file's imports, so any method named `application`/`tableView`/`dealloc` in a UIKit-importing file is rooted, whatever its class.
- **Builtin lifecycle names** (dead-code, independent of imports): `("objc", &["viewDidLoad", "applicationDidFinishLaunching"])` (roots.rs:156), plus `CONVENTIONAL_METHODS` (`initialize`, `describe`, …, roots.rs:162-170).

## 11. Test harness entries

- **`xctest`** (kind Test, rules.rs:5368-5384):
  - Gated on an objc file importing `XCTest`.
  - Roots methods named `test*` (an uppercase/non-lowercase char after `test`), `setUp` and `tearDown`.
  - Mints one harness node `<framework-entry>::test::xctest` with `entry` edges at `low` confidence (fixture f3).
  - Dead-code treats them as `test-entry` roots (`framework:test-harness`, fixture f3 JSON).
  - `--no-test-entries` turns them into root marks (generic CLI behaviour, not exercised for objc).
- **Not covered**: `+setUp`/`+tearDown` are matched by name like the instance versions (unverified distinction); `invokeTest`, `measureBlock:` blocks, Kiwi/Specta/Quick (`describe(...)`, `it(...)`), and OCMock are not covered. There is no base-type check that the class is an `XCTestCase`.
- **Builtin test-name fallback** (`is_test_name`): objc takes the default arm, `test_` or `Test` prefix (word boundary after `Test` since WS20). Obj-C's `testFoo` is matched only through the `xctest` rule, never by the builtin.
- **Test files**: `*Test(s).m`/`.mm` are test files (WS20 test-file arm).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: python/php/ruby only (rules.rs:11806). Helpers in a test class (`MyTests::helperForTest`) are live only if called.
- **Test-file gating**: `xctest` is import-detected, so the name-only test-file gate does not apply.

## 12. Dead-code analysis

- **Signals**: `ObjcPlugin` does not override `signals()`, so all are `false`: visibility, attributes, exports, test_defs, value_refs, dyn_uses, unreachable, impls (lib.rs:269-306).
  - Result: capability `class: degraded`, `max_confidence: medium`. Value references are observed when selector or function-value sites exist (fixture `lang-docs/objc/f1`, 2026-11-25: `value_references: full`, everything else `none`).
  - Findings carry `language-lacks-attributes, language-lacks-dispatch-model, language-lacks-visibility`, plus `language-lacks-value-references` when no value site exists.
- **Roots that apply**:
  - `builtin:main` (`main`, roots.rs:19).
  - `builtin:lifecycle` (`viewDidLoad`, `applicationDidFinishLaunching`, roots.rs:156, plus `CONVENTIONAL_METHODS`).
  - Framework rules (§10).
  - The `xctest` harness (§11).
  - User roots (`cgg-deadcode.toml`).
  - `ffi:export` and `lang:export` never fire (no attributes, no exports).
- **Known false positives** (fixture f1 `--dead-code`):
  - (0.11.0 only, fixed in 0.11.1: `helper`/`c_func`, called from C calls, and `Worker::onNote`, reached by `@selector`.)
  - `Service::dealloc` is flagged when UIKit is not imported by the `.m` file.
  - Protocol requirement declarations `go`/`maybe` are flagged as unreferenced methods (re-checked: `go` is still reported, `lang-docs/objc/f1`).
  - `Service::initWithName` is flagged.
  - Fixture f3: `AppVC::tapped` (`IBAction`) is flagged.
  - Fixture f2: the first `setX` overload (selector collision).
- **Confidence caps**:
  - `implements-external-contract` never applies (no override/protocol tracking).
  - `macro-generated` never applies (no macro tags).
  - `reflection` (dynuse) never applies (§15).
  - The constructor `Service::init` drops to low with `implicitly-invokable-kind, name-matches-unresolved-site` (fixture f1).
- **Unresolved-doubt**: same-name unresolved sites lower confidence generically (`name-matches-unresolved-site`, fixture f1 `Service::init`).
- **Unreachable-after-terminator**: not produced (`unreachable: false`).
- **Visibility rules / public API roots**: none. A library's public `@interface` methods in headers are not roots (headers are C and empty).

## 13. Builtins & stdlib tables

- **`stdlib/objc.txt`**: 408 lines (`wc -l`), loaded at stdlib.rs:197 and 238.
  - It mixes Foundation/UIKit selector heads (`alloc`, `init`, `addObject`, `cellForRowAtIndexPath`, `performSelector`), C functions (`NSLog`, `dispatch_async`) and type names (`CGRect`, `CGFloat`).
  - C-function entries now screen the C calls recorded since WS22 (`NSLog` → stdlib bucket, `lang-docs/objc/f1`).
- **Screening behaviour** (`external::classify_one`, external.rs:186+):
  - A receiver hint in the table → stdlib (fixture f1: `[NSNotificationCenter defaultCenter]` → stdlib; fixture f5: `[NSString stringWithFormat:]` → stdlib).
  - Anything else the screen cannot place → external (fixture f1: `alloc`, `init`, `performSelector`, `addObserver` with receivers `Service`/`self`/`[...]` → external bucket).
  - The same site can appear in both `external_calls` and `unresolved_calls` (fixture f1 `Service.m:4`, `:9`).
- **`--include-stdlib` / `--include-external`**: mint `<stdlib>::NSString::stringWithFormat` and `<external>::o::respondsToSelector`-style leaf nodes (fixture f5 mermaid).
- **`CONSTRUCTING_METHODS`** has `("objc", ["alloc"])` (stdlib.rs:59-60), and `OBJC_SELECTOR_FROM_STRING` names `NSSelectorFromString`.
- **No objc rows** in `CAST_INTRINSICS`, `TOP_TYPES`, `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS`, `CALLBACK_TYPE_CONSTRUCTORS`, or any `rust_types`-like table.
- **No implicit-package list** (`SCALA_IMPLICIT_PACKAGES` analogue).
- **`is_platform_header`** (cross_file.rs:3208+) decides which `<…>` imports are not followed (C/POSIX names). Whether Apple framework headers like `<UIKit/UIKit.h>` are classified as platform is unverified. In the fixtures no such file existed in-tree, so nothing was followed either way.

## 14. Confidence & certainty

High is reachable through:

- `[[T alloc] init…]` constructions and sends typed by a declared return type (`lang-docs/objc/f1`);
- plain C calls to a single in-closure function (`lang-docs/objc/f1` `helper`, `f2` `c_side`);
- `@selector` values when one class declares the selector;
- intra-file `self` sends to a method of the enclosing class (fixture f2 `C::doit → C::ping`; fixture f1 `Service::run → Service::helper2`);
- intra-file sends whose receiver the lowercase heuristic retyped (fixture f2 `[a ping]` → `A::ping` high, although the heuristic is marked weak);
- cross-file class-receiver sends when the class name is a unique owner (`unique_owner_names`, objc in the "module-carrying" group, cross_file.rs:4411; `qn_carries_module`, 4775). Fixture f1 `[Service create]`, fixture f3 `[Engine shared]` → high, `cross-file:imports`;
- C/C++ → objc C function through the include closure (fixture f4 `c_side → objc_side`, high).

Medium:

- every by-name fan-out on an untyped variable, parameter, block parameter or `super` (fixtures f1, f2, f3);
- cross-file sends to a variable even with a single candidate (fixture f1 `[s run]` → `Service::run` medium);
- asm → objc FFI (medium);
- same-file preprocessor alternatives (WS36).

Low: framework harness `entry` edges (fixture f3).

The mermaid `?` marker appears on arrows with no high site (fixture f5 `N2 -->|?| N0`).

Typed-receiver certainty comes only from constructions and declared return types; parameter and local declarations are not read.

## 15. Dynamic use & reflection

- **`dynuse`**: objc is not covered (`dyn_uses: false`; dynuse covers Python, JS, Java, C#, Ruby, PHP per CLAUDE.md; `grep objc plugins/dynuse.rs` → none).
- `performSelector:`, `NSSelectorFromString`, `NSClassFromString`, `respondsToSelector:`, `valueForKey:`/KVC/KVO, `NSInvocation`, `objc_msgSend` and `method_exchangeImplementations` (swizzling) produce no reflection screen and no edge.
  - `NSClassFromString(@"G")` is now recorded as a C call (stdlib-screened), but names no class.
  - Sends like `[o performSelector:…]` land in the external bucket.
- **`@selector(...)` / `NSSelectorFromString(@"…")`** with a literal are value references (§03), not reflection screens.
- **Module loads** (`dlopen`, `NSBundle load`): not modelled.

## 16. Known gaps & limitations

- ~~C function calls are never recorded in `.m`/`.mm`~~: fixed in 0.11.1 (WS22, §03). A block invocation through a field (`self.blk(1)`) is still not recorded.
- **Selectors are truncated to the first keyword** (objc.rs:176-180, 111-114). Distinct selectors sharing a head collide (`setX:y:`/`setX:z:`), and edges go to an arbitrary one (fixture f2).
- **`+`/`-` methods of one selector share a qn**, so `ambiguous-in-file` (fixture f5).
- **No base classes or protocol conformance recorded**:
  - inherited `self` sends are unresolved (fixture f1);
  - `super` fans out to unrelated classes (fixture f2);
  - `--dynamic-dispatch` is a no-op;
  - documented in cross_file.rs:4086-4088 and CHANGELOG.md:458-460.
- **Protocol members are owner-less** (tagged `abstract` since WS22).
- **Little type inference**: beyond the lowercase-name heuristic, constructions and a send's declared return type, parameters, declared locals, casts, `id<P>` and properties are all ignored (§05).
- **Obj-C headers are parsed as C**: `@interface`/`@protocol`/`@property` in `.h` are lost, and their framework imports never activate objc rules (fixtures f1, f4).
- **`.mm` uses the Obj-C grammar**: C++ classes, namespaces and calls are lost (fixture f3).
- **No `@property`/`@synthesize` accessor nodes; dot syntax is not a call** (fixtures f2, f6).
- **No macro support**: no `#define` callables and no `c_macro::expand` (lib.rs:527-531) (fixtures f2, f6).
- **Computed `performSelector`, KVC, `IBAction`/storyboard wiring are invisible** (literal `@selector` is read since WS26). They are disclosed by `objc-selector` and by the `xctest` gap text, which is stale.
- **No dead-code signals at all**: degraded class, `max_confidence: medium` (fixture f3).
- **Test-only coverage** (0.11.0): 3 inline unit tests (objc.rs has more since) and one detection probe (cgg/tests/detect_prefixes.rs:52-57). There is no objc resolve test under `crates/*/tests`. The `"Foo.m"` strings in `crates/cgg/tests/resolve.rs:3144-3190` are qualified names (`Foo.m`) from other languages' tests, not Obj-C files.
- **Benchmark**: one corpus repo, AFNetworking (scripts/benchmark.sh:73; README.md:1325 "AFNetworking | objc | 288 | 93 | 5%"). App rows `app-orleans-samples` and `app-rn-expensify` list objc rules as `~` (not expected to fire) (benchmark.sh:209-210).
- **README language table** row (README.md:499): "Objective-C | #import | — | Message expressions": no type inference claimed.
- **Odd resolution observations** (cause not investigated):
  - `[[Service alloc] init]` stayed unresolved in 0.11.0 (fixture f1); constructions now bind (§03);
  - duplicate unresolved records for one block-nested send (fixture f1 `main.m:13`).

## Summary scorecard

| section | support | one-line note |
|---|---|---|
| 01. Detection & parsing | partial | `.m`/`.mm` only; `.h` with `@interface` → C; `.mm` C++ mis-parses; depth skip works |
| 02. Callables extracted | partial | methods, class methods, `init`, categories, C functions; selectors truncated, `+`/`-` collide, no properties/blocks/macros |
| 03. Call-site forms | partial | message sends, C calls, `alloc`/`init` constructions, `@selector` and C function values; no dot-syntax, no arity, casts not peeled |
| 04. Imports & module resolution | partial | `#import`/`#include` closure + family retry + prototype unification; no external-module refusal, no `@import`, sends not import-scoped |
| 05. Type inference & receiver typing | minimal | `self`, class-name receivers, constructions, a send's declared return type and the same-file lowercase-name heuristic |
| 06. Inheritance, dispatch & overloading | none | no bases, no conformance (protocol members only tagged `abstract`), `super` fans out by name, no dynamic dispatch |
| 07. Scoping & shadowing | minimal | no locals table; same-file preprocessor alternatives bind at medium |
| 08. Macros, preprocessor & codegen | none | no `#define` callables, excluded from `c_macro::expand` |
| 09. FFI & cross-language links | partial | C/C++/asm ↔ objc C functions work both ways through the include closure |
| 10. Frameworks & entry points | partial | 4 matching rules (UIKit/AppKit lifecycle, app delegate, Cocoa delegates, XCTest) + 2 gap rules; name-only, header imports ignored |
| 11. Test harness entries | partial | XCTest `test*`/`setUp`/`tearDown` via harness node; no base check, no Kiwi/Specta |
| 12. Dead-code analysis | minimal | degraded class, no declared signals, max medium; C calls and selectors now credit |
| 13. Builtins & stdlib tables | partial | 408-name `objc.txt` screen; no cast/constructor/nullability tables |
| 14. Confidence & certainty | partial | high for `self`/unique class receivers, constructions, C calls and selectors; fan-out medium |
| 15. Dynamic use & reflection | none | no dynuse; literal selectors are references; KVC/`NSClassFromString` invisible |
| 16. Known gaps & limitations | n/a | selector truncation, no inheritance, headers-as-C are the headline gaps |
