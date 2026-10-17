# 02. Callables extracted

This category covers which definition shapes a language plugin turns into a graph node (a `DefRecord`, later a `CallableNode`), and how each node gets its name, owner and kind. It includes free functions, methods, constructors and destructors, static or class methods, properties and accessors, operator definitions, abstract and interface members, overloads and clauses, nested and local definitions, named and anonymous closures, macro-generated code, bodiless prototypes, and the qualified name (QN) with its module or owner path. It also covers the per-definition metadata that later phases read: visibility, attributes, and the variant/kind. A shape that produces no node cannot be the source or target of an edge. Its calls are attributed to the nearest enclosing callable, or they are recorded as `no-enclosing-callable`. This makes the category the base for every later one, from call sites (03) to dead code (12).

Sources: section `## 02. Callables extracted` and the Summary-scorecard row 02 of each of the 46 files in `dev_docs/languages/`. The framework catalogue `dev_docs/frameworks/rules-part-{1..4}.md` was used for the route-handler gap. Disputed or surprising claims were re-checked in code (cited `file:line`) or by running `target/release/cgg 0.11.0 -t json` on small fixtures under `scratchpad/parity/matrix-02/{a,b,c,d,e}/`, cited as **fx-a** … **fx-e**.

## Legend

| Mark | Meaning |
|---|---|
| ✅ | full: the common shapes of the concept become correctly named nodes |
| 🟡 | partial: some shapes are extracted, or they are extracted with a wrong or colliding name, owner or kind |
| ⬜ | none: the concept exists in the language, but no node or metadata is produced |
| — | n/a: the language has no such concept |

### Columns

| Column | Capability | ✅ requires |
|---|---|---|
| **Fn** | Free or top-level function. For descriptor and config languages, the top-level definition unit (shape, type, labelled block, entity, label) | top-level definitions become nodes |
| **Meth** | Member functions with the owner (class, impl, table, contract, service) in the QN | members are owned by their real owner |
| **Ctor** | Constructors, initializers, destructors and finalizers | a node exists, with a `Constructor`/`Destructor` variant or an equivalent named node |
| **Static** | Static or class methods distinguished (`StaticMethod` variant) | the variant is set |
| **Acc** | Properties, getters, setters, computed properties, subscripts and generated accessors | accessor bodies are nodes |
| **Op** | Operator-overload definitions (`operator+`, `__add__`, `def +`, `static func +`) | nodes are extracted |
| **Abs** | Abstract, interface, protocol or trait member declarations | extracted **and** recognised as declarations (tag `abstract`, `@abstractmethod`, Protocol/ABC base; `cgg-resolve/src/cross_file.rs:1152-1190`). Extracted but untagged is 🟡 |
| **Ovl** | Overloads, clauses and multiple-dispatch methods | one node each **and** grouped as one callee (`binds_same_name_set`, `cgg-resolve/src/intra_file.rs:835-850`). Separate but ungrouped is 🟡 |
| **Nest** | Nested or local named functions | extracted with the enclosing function in the QN. Extracted with a flat QN is 🟡 |
| **NClo** | A lambda or closure bound to a name (`const f = () =>`, `let f = \|x\|`,`f <- function`) | a node named after the binding |
| **Anon** | Anonymous closures given a synthetic node (`handler_at_<line>` route handlers, `closure_at_<line>` spawned closures, KV `kv_suite`/`kv_rule`) | — (no language has full coverage) |
| **Gen** | Macro-defined or macro-generated definitions and generated code (`macro_rules!`, `#define`, `defmacro`, tagged `macro:<name>`) | definitions and expansion supported |
| **Decl** | Bodiless prototypes and foreign declarations (`extern`, prototypes, `.pyi`, abstract or interface bodies, `axiom`) | kept as nodes, and unified with bodies where the language has both |
| **ModQN** | The module, package or namespace path is part of the QN | ✅ when it is |
| **Vis** | Visibility recorded (`DefRecord::vis` / `visibility`) | ✅ when it is |
| **Attr** | Annotations, decorators or attributes recorded on the definition (`DefRecord::attributes`) | ✅ when they are |
| **Init** | Code outside any function becomes a callable: initializer blocks, `init {}`, top-level script statements, `test` blocks, field initializers | — (no language has full coverage) |
| **Kind** | The variant/kind is correct: no function reported as a method or the reverse, no spurious nodes for non-functions | ✅ when it is correct |

## The matrix

| Language | Fn | Meth | Ctor | Static | Acc | Op | Abs | Ovl | Nest | NClo | Anon | Gen | Decl | ModQN | Vis | Attr | Init | Kind | Most important gap |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|---|
| rust | ✅ | ✅ | — | ⬜ | — | ✅ | 🟡 | — | 🟡 | 🟡 | 🟡 | 🟡 | ✅ | ✅ | ✅ | ✅ | — | ✅ | nested `fn` is filed as a *method* of the enclosing impl (`H::nested_inner`, kind method) and named closures take the impl/module scope, so both collide with real members |
| python | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | 🟡 | ✅ | 🟡 | ⬜ | ⬜ | ✅ | ✅ | ✅ | ✅ | ⬜ | 🟡 | `@x.setter`/`deleter` become a second same-QN *method*; `async def` methods get kind `function`; no node for anonymous lambdas or `self.f = lambda` |
| kivy | — | 🟡 | — | — | — | — | — | — | — | — | 🟡 | — | — | — | — | — | 🟡 | ✅ | `canvas:` blocks, `[Tmpl@Base]` templates, 2nd+ selectors of `<A,B>` and KV inside `Builder.load_string` produce no node |
| javascript | ✅ | 🟡 | ✅ | ✅ | ✅ | — | — | — | 🟡 | 🟡 | 🟡 | — | — | ⬜ | ✅ | ✅ | ⬜ | 🟡 | class fields holding functions (`field = () => …`) are not callables, so their calls have no enclosing callable; QNs carry no module, and object-literal / prototype members are owner-less |
| typescript | ✅ | 🟡 | ✅ | ✅ | ✅ | — | ✅ | ✅ | 🟡 | 🟡 | 🟡 | — | ⬜ | ⬜ | ✅ | ✅ | ⬜ | 🟡 | class-field arrows (React/Angular handlers) are not callables; `namespace NS {}` is not a scope (`NS.inNs()` unresolved); `declare`/`.d.ts` ignored |
| go | ✅ | ✅ | — | — | — | — | 🟡 | — | 🟡 | 🟡 | 🟡 | — | ✅ | 🟡 | ✅ | — | 🟡 | ✅ | package-level / `var` func literals get no node; `a, b := func(){}, func(){}` mints one bogus node `a, b`; interface specs are not tagged `abstract` |
| java | — | ✅ | ✅ | ✅ | — | — | 🟡 | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | lambdas, `static {}`/instance initializers and field initializers are not callables (`no-enclosing-callable`); anonymous-class methods collide with the enclosing class's QN |
| kotlin | 🟡 | ✅ | 🟡 | — | ⬜ | ✅ | 🟡 | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | 🟡 | ✅ | ✅ | ✅ | 🟡 | 🟡 | property accessors and every lambda (`val lam = {}`) have no node; top-level functions in a package get kind `method`; interfaces get a constructor node |
| c | ✅ | — | — | — | — | — | — | — | 🟡 | — | — | ✅ | ✅ | — | ⬜ | ⬜ | — | ✅ | no visibility (`static` linkage) and no `__attribute__`/`__declspec` recorded; prototypes whose parameter text differs from the body stay duplicate nodes |
| cpp | ✅ | ✅ | ✅ | ⬜ | — | 🟡 | 🟡 | ✅ | — | ⬜ | ⬜ | ✅ | ✅ | ✅ | ⬜ | 🟡 | — | 🟡 | lambdas are never callables; conversion operators and explicit full specialisations are dropped; namespace functions have kind `method`; inline friends owned by the class |
| csharp | 🟡 | ✅ | ✅ | ⬜ | 🟡 | ⬜ | 🟡 | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | 🟡 | 🟡 | ✅ | ✅ | ⬜ | 🟡 | operator and conversion-operator declarations are not extracted; a file-scoped `namespace X;` is missing from every QN; accessors are named `get`/`set` |
| bash | ✅ | — | — | — | — | — | — | — | 🟡 | — | — | ⬜ | — | — | — | — | ⬜ | ✅ | names are flat (nested `inner` is just `inner`) and top-level script code is not a callable |
| ruby | ✅ | ✅ | ✅ | 🟡 | ⬜ | ✅ | — | — | ✅ | ⬜ | 🟡 | ⬜ | — | 🟡 | ⬜ | — | ⬜ | 🟡 | `attr_accessor`, `define_method`, `alias_method` generate nothing; no visibility; `class Shop::X` inside `module Shop` doubles the prefix |
| swift | ✅ | ✅ | 🟡 | 🟡 | ⬜ | ✅ | ✅ | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | computed properties, accessors, subscripts and `deinit` have no node (SwiftUI `var body` is invisible); no visibility or attributes recorded |
| lua | ✅ | ✅ | — | — | — | ✅ | — | — | 🟡 | ✅ | ⬜ | — | — | ⬜ | ⬜ | — | ⬜ | ✅ | anonymous functions and `return { f = function() end }` module tables are not callables; `M.f` and `M:f` are different QNs |
| php | ✅ | ✅ | ⬜ | ⬜ | ⬜ | — | 🟡 | — | ✅ | ⬜ | ⬜ | — | 🟡 | ⬜ | ⬜ | ✅ | ⬜ | 🟡 | closures and arrow functions are never callables (Laravel route closures get no node); `__construct` is kind `method`; namespace absent from QNs |
| dart | ✅ | 🟡 | 🟡 | ⬜ | 🟡 | ✅ | ✅ | — | ⬜ | ⬜ | ⬜ | 🟡 | ✅ | ⬜ | ⬜ | ⬜ | — | 🟡 | bodiless constructors (`Square(this.side);`, the common form) are not callables; enum/extension members owner-less; local functions not recorded |
| scala | ✅ | 🟡 | 🟡 | — | 🟡 | ✅ | ✅ | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ⬜ | ✅ | ⬜ | 🟡 | `val f = …` function values, lambdas and primary constructors are not callables; the package never enters the QN; every def is variant FreeFunction |
| hcl | 🟡 | — | — | — | — | — | — | — | 🟡 | — | — | — | — | ⬜ | — | — | ⬜ | — | only blocks with ≥2 labels are nodes; nested blocks are flat; `locals`, `.tfvars` and unlabelled blocks invisible |
| zig | ✅ | 🟡 | — | — | — | — | — | — | 🟡 | — | — | — | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | container members carry no owner in their QN (`Counter.inc` is `inc`); `test` and `comptime` blocks are not callables; no `pub`/`export` |
| objc | ✅ | 🟡 | 🟡 | ✅ | ⬜ | — | 🟡 | — | — | ⬜ | ⬜ | ⬜ | ✅ | — | ⬜ | ⬜ | — | ✅ | multi-keyword selectors are truncated to the first keyword (`addA:toB:` → `addA`), so `setX:y:`/`setX:z:` collide; no `@property` accessors |
| r | ✅ | ⬜ | — | — | — | 🟡 | — | — | 🟡 | ✅ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | ⬜ | ✅ | R6 / Reference-class / S4 `setMethod` / `obj$f <- function` methods produce no node |
| groovy | ✅ | ✅ | ⬜ | ✅ | ⬜ | ✅ | 🟡 | ✅ | — | ⬜ | ⬜ | ⬜ | 🟡 | ✅ | ⬜ | ✅ | ⬜ | ✅ | constructors are not extracted (Java's are); closures are never callables (`closure_expression` arm names a non-existent kind); traits, Spock features |
| julia | ✅ | — | 🟡 | — | — | ⬜ | — | ✅ | 🟡 | ⬜ | ⬜ | 🟡 | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ✅ | `macro` definitions, functor methods, operator methods and `Base.f(x) = …` short forms are not extracted; named lambdas (`sq = x -> …`) neither |
| perl | ✅ | 🟡 | — | — | ⬜ | ⬜ | — | — | ✅ | ⬜ | 🟡 | ⬜ | 🟡 | 🟡 | — | ⬜ | ⬜ | ✅ | anonymous subs (except route handlers), Moose `has` accessors, `method`/`func` keywords; `package Blk { }` scope leaks to later subs |
| elixir | ✅ | — | — | — | — | ⬜ | 🟡 | ✅ | — | ⬜ | 🟡 | 🟡 | 🟡 | ✅ | ⬜ | ⬜ | ⬜ | ✅ | `defprotocol`/`defimpl` give no owner (two owner-less `size`); `defp` is not recorded as private; operator defs misnamed (`def a + b` → `a`) |
| erlang | ✅ | — | — | — | — | — | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ⬜ | — | ✅ | funs (incl. named funs), `-callback`/`-spec` and `-define` macro-generated functions produce no node |
| fortran | ✅ | ⬜ | — | — | — | ⬜ | 🟡 | ⬜ | 🟡 | — | — | — | 🟡 | 🟡 | ⬜ | ⬜ | — | 🟡 | the module prefix leaks past `end module` (`m1::after_mod`); type-bound, generic, `entry`, statement and separate-module procedures missing |
| clojure | ✅ | ⬜ | — | — | — | — | ⬜ | 🟡 | ⬜ | ✅ | 🟡 | 🟡 | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | 🟡 | name metadata is kept in the name (`app.util/^:private hidden`), so calls miss; `defprotocol`/`deftype`/`reify` members are not callables |
| haskell | ✅ | 🟡 | ⬜ | — | ⬜ | ⬜ | ⬜ | ✅ | 🟡 | ✅ | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | — | — | 🟡 | type variables in signatures (`a -> b`) and `x <- act` do-binds mint spurious callables; operators, data constructors, class method signatures missing |
| ocaml | ✅ | ⬜ | — | — | — | ⬜ | ⬜ | — | 🟡 | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | 🟡 | the module name is never set (`extract_module` looks for a direct `module_name` child), so every QN is bare; plain values and `let _ =` are over-extracted |
| powershell | ✅ | ✅ | ✅ | ✅ | — | ✅ | — | 🟡 | 🟡 | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | script top level and script blocks (`$sb = { }`, Pester `It { }`) are not callables; `global:`/`script:` prefixes kept in names |
| solidity | ✅ | ✅ | 🟡 | — | ⬜ | — | 🟡 | ✅ | — | — | — | — | 🟡 | 🟡 | ✅ | 🟡 | — | ✅ | `fallback()`/`receive()`, public state-variable getters and Yul functions get no node; interface members untagged |
| fsharp | ✅ | 🟡 | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | ✅ | 🟡 | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ✅ | module/namespace never in the QN (`Util.only` and `Other.only` are both `only`); operators, active patterns, constructors, abstract slots missing |
| starlark | ✅ | — | — | — | — | — | — | — | 🟡 | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | ⬜ | ✅ | `my_rule = rule(...)` / `macro(...)` targets are not callables; QNs carry no `.bzl` path |
| cmake | ✅ | — | — | — | — | — | — | — | 🟡 | — | — | — | — | — | — | ⬜ | ⬜ | 🟡 | `macro()` is indistinguishable from `function()`; quoted and `${var}` names kept verbatim |
| nix | 🟡 | — | — | — | — | — | — | — | 🟡 | ✅ | ⬜ | — | — | ⬜ | — | — | ⬜ | ✅ | the file-level lambda (`{ config, lib, ... }: { }`, `{ stdenv }: mkDerivation`) is never a node |
| verilog | ✅ | 🟡 | ⬜ | — | — | — | ⬜ | — | — | — | — | — | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | 🟡 | SystemVerilog `class` (owner-less methods), `package`, `interface`, `program`, `function new` and DPI imports not extracted |
| vhdl | ✅ | 🟡 | — | — | — | ⬜ | — | 🟡 | 🟡 | — | — | — | ⬜ | ⬜ | — | 🟡 | ⬜ | 🟡 | package and protected-type members are unqualified; processes, operator functions and subprogram declarations missing; overloads not grouped |
| asm | 🟡 | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | ⬜ | 🟡 | — | 🟡 | labels matching `meta_ident` (`.loop:`), `ENTRY()`/`SYM_FUNC_START`, `PROC`, `.macro` missed; every label is a 'function' up to the next label |
| smithy | 🟡 | ⬜ | — | — | — | — | — | — | ⬜ | — | — | — | — | ✅ | — | ⬜ | — | 🟡 | members and inline `input := {}` structures are not nodes; IDL 1.0 `set`, `bigInteger`/`bigDecimal` dropped |
| proto | ✅ | ✅ | — | — | — | — | — | — | ⬜ | — | — | — | — | ✅ | — | ⬜ | — | ✅ | nested `message`/`enum` declarations are never extracted (refs to them go unresolved); `extend` blocks |
| graphql | ✅ | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | 🟡 | fields, `directive` definitions, `schema`, `extend type` and executable operations/fragments produce no node |
| openapi | ✅ | 🟡 | — | — | — | — | — | — | ⬜ | — | — | — | — | ⬜ | — | — | — | 🟡 | QNs carry no section, so `schemas/Dup` and `responses/Dup` collide; Swagger 2 `parameters`/`responses`, `webhooks`, `callbacks`, `pathItems` missing |
| asyncapi | ✅ | 🟡 | — | — | — | — | — | — | ⬜ | — | — | — | — | ⬜ | — | — | — | 🟡 | AsyncAPI 2 `operationId`s and channel-local messages are not nodes; same-named message and schema collide |
| lean | ✅ | 🟡 | ✅ | — | ✅ | ⬜ | 🟡 | — | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | ✅ | 🟡 | ✅ | ⬜ | ✅ | `where` auxiliaries and `let rec` are not callables; anonymous instances lose their bodies; `open X in def …` drops the def entirely |

### Column totals (46 languages)

| Column | ✅ | 🟡 | ⬜ | — |
|---|--:|--:|--:|--:|
| Fn | 38 | 6 | 0 | 2 |
| Meth | 15 | 15 | 6 | 10 |
| Ctor | 9 | 7 | 5 | 25 |
| Static | 7 | 2 | 6 | 31 |
| Acc | 3 | 5 | 9 | 29 |
| Op | 10 | 2 | 10 | 24 |
| Abs | 5 | 13 | 6 | 22 |
| Ovl | 14 | 4 | 1 | 27 |
| Nest | 4 | 25 | 7 | 10 |
| NClo | 6 | 5 | 19 | 16 |
| Anon | 0 | 9 | 22 | 15 |
| Gen | 2 | 5 | 18 | 21 |
| Decl | 12 | 8 | 9 | 17 |
| ModQN | 14 | 6 | 19 | 7 |
| Vis | 9 | 1 | 23 | 13 |
| Attr | 12 | 5 | 17 | 12 |
| Init | 0 | 3 | 28 | 15 |
| Kind | 22 | 23 | 0 | 1 |

Two columns have no ✅ in any language. **Anon** has none because every language that mints synthetic closure nodes does so only for one call shape (a registrar route, a `spawn`). **Init** has none because no plugin makes a callable out of top-level, initializer or test-block code. Only six plugins write `handler_at_` nodes: elixir, clojure, go, javascript (TS too), ruby and perl (`grep -ln handler_at_ crates/cgg-lang/src/plugins/*.rs`). Rust's `spawn` closures (`rust.rs:1340-1446`) and Kivy's `kv_suite`/`kv_rule` (`kivy.rs:363-374, 715-773`) are the only other synthetic nodes.

## Evidence per row

In this section, "doc" means `dev_docs/languages/<lang>.md` §02. Plugin paths are under `crates/cgg-lang/src/plugins/`. Only the 🟡, ⬜ and surprising cells are argued; a ✅ cell is backed by the doc's "Supported" list.

- **rust**:
  - Variants are set at `rust.rs:767-773`. The code produces no `StaticMethod`; the only mention is a match arm at rust.rs:1405, and associated functions are `InherentMethod` (doc).
  - Trait declarations are `TraitDefaultMethod` and are not tagged `abstract`. The `abstract` tag is pushed only in scala.rs:196, dart.rs:119, swift.rs:209 and javascript.rs:370.
  - Nested `fn` and named-closure QNs: **fx-b** gives `crate::H::nested_inner` with kind `method`, and `crate::H::f` with kind `closure` for a closure inside `H::run`.
  - Gen 🟡: arms with repetition and proc-macros are not expanded (doc; `rust_macro.rs`).
  - Vis/Attr: `rust_vis` and `collect_attributes` (rust.rs:806-828, 2905).
- **python**:
  - Ctor/Static/Prop: `python.rs:512-521`.
  - Setter: **fx-a** gives `a.A.p` once as `property` and once as `method`.
  - Async method: **fx-a** gives `a.A.am` with kind `function`.
  - Abs ✅: `@abstractmethod` and Protocol/ABC/ABCMeta bases (`cross_file.rs:1161-1186`).
  - Ovl 🟡: the `@overload` stubs are separate same-QN nodes (doc, fixture f2), but `python` is not in `binds_same_name_set` (`intra_file.rs:835-850`).
  - Vis: `py_vis`, from the underscore convention (`python.rs:1151-1159`).
  - Init ⬜: there is no `<module>` callable (doc).
  - Anon ⬜: python.rs has no `handler_at_`.
- **kivy**:
  - Every node is an `InherentMethod` `<Owner>.on_x:<line>`. Only the first selector is used (`kivy.rs:154`).
  - Anon/Init 🟡: `kv_suite` and `kv_rule` nodes exist, but `canvas:` has none (doc; `deadcode/caps.rs:59-66`).
- **javascript**:
  - QNs are bare, with no module (`javascript.rs:151-157`; `cross_file.rs:5605-5608`).
  - Static: `StaticMethod` (the variant grep counts 2 in javascript.rs).
  - Class fields are not callables, and the anonymous IIFE, `static {}` and top-level code have no node (doc fixtures f1, f4, f9).
  - Vis: `class_member_vis` (javascript.rs:1364).
- **typescript**:
  - Abs ✅: `abstract` tag (javascript.rs:370; test `interface_and_abstract_members_are_bodiless_declarations`).
  - Ovl ✅: the one implementation body is the callable and `typescript` is in `binds_same_name_set`.
  - Namespaces: **fx-b** gives `inNs` instead of `NS.inNs`, and the class field `handler = () => …` has no node.
  - Decl ⬜: `declare` and `.d.ts` are not extracted (doc).
- **go**:
  - Interface specs are `InherentMethod`, untagged (go.rs:405-456; not in the `abstract` grep).
  - Multi-assignment: **fx-c** gives the node `w.a, b`. The package-level `var Hook = func(){}` gives no node (**fx-c**).
  - Decl ✅: **fx-e** gives `w.asmImpl` for a bodiless `func asmImpl(x int) int`.
  - ModQN 🟡: the package clause is used, not the import path (go.rs:293, 340-344).
  - Init 🟡: `func init` is a function (**fx-e** `w.init`), but package-var initializers are not callables.
- **java**:
  - `StaticMethod` from the keyword token (java.rs:236-247).
  - Ovl ✅: java is in `binds_same_name_set`.
  - Abstract and interface members are untagged (not in the grep).
  - Lambdas and initializers are not callables (doc; `java-runtime` gap rules.rs:7144-7158).
  - Annotation elements are not extracted (java.rs:108-111).
- **kotlin**:
  - Top-level functions in a package: **fx-a** gives `p.top` with kind `method` (kotlin.rs:327-333).
  - Interface constructor: **fx-a** gives `p.Shape constructor`.
  - `init {}` and secondary constructors are folded into the class-as-constructor span (doc fixture f8), so Init and Ctor are 🟡.
  - Accessors and lambdas have no node (doc fixture f8).
  - Abs: `p.Shape.area` is an untagged `method` (**fx-a**).
- **c**:
  - QNs are flat and there are no owners (c.rs:206-222).
  - `visibility` is always `""` and `attributes` is empty (c.rs:215-216, 246-247).
  - Gen ✅: fn-like `#define` nodes and one-level expansion (c_macro.rs:779-895).
  - Decl ✅: `unify_declarations` (cpp.rs:1335+).
- **cpp**:
  - Variants are set at cpp.rs:393-399. **fx-a** gives `app::free_fn` with kind `method`, has no node for `operator bool`, and has no lambda node.
  - Static ⬜: there is no `StaticMethod` in cpp.rs.
  - Attr 🟡: only the CUDA qualifiers and `virtual` (cpp.rs:937-944, 2713-2729).
  - Vis ⬜: always empty (cpp.rs:346).
- **csharp**:
  - **fx-a** (`namespace Acme.App;`) gives `Svc.Helper` with no namespace, and has no node for `operator +` or `implicit operator int`.
  - The accessor node is named `Svc.get` (csharp.rs:230-240).
  - Static ⬜: there is no `StaticMethod` in csharp.rs.
  - Minimal-API lambdas: **fx-e** `app.MapGet("/", () => Helper())` produced only `Helper`, even though the `aspnet-minimal` rule lists `MapGet` as a registrar (rules-part-1.md, rules.rs:1178).
- **bash**: every node is a flat `FreeFunction` (bash.rs:87-112). Neither `eval`-defined functions nor the top level produce a node (doc).
- **ruby**:
  - Fn: **fx-c** gives `top_helper` with kind `method`.
  - Ctor: **fx-c** gives `S::initialize` as a `constructor`.
  - Static 🟡: `def self.c` gives `StaticMethod` (ruby.rs:214-219), but the JSON kind is `method` and `class << self` is not detected (doc).
  - Acc ⬜: `attr_accessor` gives nothing (**fx-c**).
  - Vis ⬜: hard-coded empty (ruby.rs:305).
- **swift**:
  - `class func`: **fx-b** gives `Sq.klass` as a plain `method`.
  - Abs ✅: the requirement `P.area` is tagged (swift.rs:209).
  - Acc/deinit ⬜: **fx-b** has no node for `var x { get }`, `deinit` or `subscript`.
  - Vis and attributes are empty (swift.rs:113-114, 178-179).
- **lua**: no scope is ever pushed (lua.rs:37, 54-60), so nested QNs are flat; `visibility` and `attributes` are empty (lua.rs:226-227, 247-248); metamethods are ordinary members (doc).
- **php**:
  - Ctor ⬜: **fx-c** gives `Ctor::__construct` with kind `method`.
  - Closures: **fx-c** `$f = function(){}` has no node. php.rs has no `anonymous_function` arm (doc), and the `laravel` rule registers `get/post/…` (rules.rs:1035).
  - ModQN ⬜: the namespace lives only in `PhpIndex` (libscope.rs:2010-2050).
  - Vis ⬜: empty (php.rs:239).
  - Decl 🟡: interface and abstract members are extracted untagged.
- **dart**:
  - Ctor 🟡: there is no `DefVariant::Constructor` in dart.rs. Constructors with a body are `InherentMethod` (dart.rs:151), and bodiless ones are dropped (doc fixture f1).
  - Static ⬜: **fx-e** gives `Sq::build` with kind `method`.
  - **fx-b** gives `paint` with no `Color::` owner. `Square.unit`, the `Square(this.side)` constructor and the redirecting forms have no node.
  - Vis and attributes are empty (dart.rs:266-267).
- **scala**:
  - Everything is `FreeFunction` except abstract declarations, which are `TraitMethod` (scala.rs:178, 194). That makes Kind 🟡.
  - Abs ✅: scala.rs:196.
  - Ovl ✅: in `binds_same_name_set`.
  - The package is never in the QN (scala.rs:64-70).
  - Vis: empty (scala.rs:355).
- **hcl**: requires `labels.len() >= 2` (hcl.rs:101); the walker tracks no scope (hcl.rs:57-60).
- **zig**:
  - QN = the simple name. Membership lives only in `type_spans` (zig.rs:158-203, 335-360). **fx-d** gives `inc` for `C.inc`.
  - The `test` block has no node (**fx-d**).
  - Vis and attributes are empty (zig.rs:200-201).
- **objc**:
  - Selector truncation: **fx-c** gives `A::addA` for `addA:toB:` (objc.rs:176-180).
  - Ctor 🟡: only `init` is a constructor (objc.rs:187-188).
  - Protocol members are owner-less and untagged (objc.rs:61).
  - There is no `preproc_function_def` arm.
- **r**:
  - Only an `identifier` LHS counts (r.rs:94-96). `obj$f` assignments, R6, RC and S4 methods have no node (doc).
  - NClo ✅: every R function is a lambda bound to a name (**fx-e** `fn = function(x) x` gives the node `fn`).
- **groovy**:
  - Ctor ⬜: **fx-a** `Plain() { init() }` has no node.
  - Closures: the `closure_expression` arm (groovy.rs:129) names a kind that the grammar does not have. `tree-sitter-groovy-0.1.2/src/node-types.json` has only `closure`.
  - Static ✅: groovy.rs:163.
  - Ovl ✅: in `binds_same_name_set`.
  - Vis ⬜: groovy.rs:187.
- **julia**:
  - Every node is a `FreeFunction` QN'd by module (julia.rs:55-77).
  - Gen 🟡: `macro_definition` has no fields in grammar 0.23.1 (checked in `node-types.json`: no fields; its children are `signature`, …), so `macro twice` is not extracted, while `@inline` and `@generated` definitions are (doc fixture p4).
  - Named lambda: **fx-e** `sq = x -> x*x` has no node.
  - Ovl ✅: `julia` is in `binds_same_name_set`.
- **perl**:
  - All `FreeFunction` `Pkg::name` (perl.rs:68-74, 302-321).
  - Decl 🟡: **fx-c** gives the phantom node `Foo::forward_decl`.
  - Block-package leak (perl.rs:94-96).
  - Anon 🟡: route handlers only (perl.rs:658-703).
- **elixir**:
  - Ovl ✅: clauses (`is_clause_language`, intra_file.rs:858-859).
  - Protocols: **fx-b** gives two owner-less `size` nodes for `defprotocol Size`/`defimpl`.
  - Vis and attributes are empty (elixir.rs:334-347), so `defp` is not private.
  - Gen 🟡: `defmacro` is a node, but `quote` defs are mis-owned and `unquote` is skipped (elixir.rs:324-328).
- **erlang**: one node per clause, grouped (erlang.rs:104-107, 191-217; `is_clause_language`). The doc reports that `-callback`, `-spec`, funs and `?MKFUN` produce no node (fixtures f6, f8).
- **fortran**:
  - Module-prefix leak: **fx-d** gives `m1::after_mod` (fortran.rs:99-110; nothing resets it).
  - Kind 🟡: PROGRAM, SUBROUTINE and FUNCTION are indistinguishable (rules.rs:6763 gap).
- **clojure**:
  - **fx-d** gives `app.util/^:private hidden`, from `named_child(1)` taken verbatim (clojure.rs:216-217).
  - **fx-d** gives `app.util/Speaker` only; its member `speak` is not a node.
  - Multi-arity `defn` is one node, so Ovl is 🟡 (doc).
  - NClo ✅: any `def` binding counts (clojure.rs:101-123). That is also an over-extraction.
- **haskell**:
  - **fx-b** gives the spurious `Data.Thing.a` from `areaOf :: a -> Double`, and no node for the class method signature itself.
  - Ovl ✅: equations are clauses (`is_clause_language`).
- **ocaml**:
  - **fx-a** gives `mfun` instead of `M.mfun`. `extract_module` looks for a direct `module_name` child (ocaml.rs:127-137).
  - **fx-a** gives the node `value` for `let value = 42`, and none for `( +! )`.
- **powershell**:
  - Ctor/Static: powershell.rs:172-197.
  - Ovl 🟡: redefinitions and class-method overloads are separate nodes, but `powershell` is not in `binds_same_name_set`.
  - Attributes `[]` (powershell.rs:157, 216).
- **solidity**:
  - **fx-c** has no node for `fallback()` or `receive()`; only `T.f` appears. That makes Ctor 🟡 (special functions).
  - Vis ✅: solidity.rs:206-220.
  - Attr 🟡: only `library-member` (solidity.rs:232).
  - ModQN 🟡: QNs are contract-scoped, but file-level functions are bare (doc fixture f6).
- **fsharp**:
  - Only the class `anon_type_defn` pushes scope (fsharp.rs:75-89). **fx-d** gives `only` for `module Lib.A`, and the owner-less `Double` for a record member.
  - Value bindings are skipped (fsharp.rs:138-141).
  - Ovl ✅: in `binds_same_name_set`.
- **starlark**: def only, bare QN (starlark.rs:82-108); lambdas and `rule()` results are not callables (doc).
- **cmake**: `function` and `macro` both become `FreeFunction` with no tag (cmake.rs:58-67, 111-123).
- **nix**: only a `binding` whose value is a `function_expression` counts (nix.rs:73-76), so the file lambda is not a node (`nix` gap, rules.rs:6692).
- **verilog**: **fx-d** gives the owner-less `run` from an SV `class Base` (verilog.rs:58-64; only `module_declaration` pushes scope). Attr 🟡: only the `module`/`task`/`function` tags (verilog.rs:88-157).
- **vhdl**:
  - Operator designators return early (vhdl.rs:220-226).
  - Package bodies push no scope (doc fixture a).
  - `vhdl` is not in `binds_same_name_set`.
- **asm**: `record_label` needs an `ident` child (asm.rs:123-127), and the spans are stretched to the next label (asm.rs:42-63).
- **smithy**: shapes are `FreeFunction` named `ns#Name` (smithy.rs:157-201); there are no member nodes.
- **proto**: `walk` returns after recording a top-level message (proto.rs:89-96); rpcs are `InherentMethod` (proto.rs:131-165).
- **graphql**: `DEF_KINDS` lists only six kinds (graphql.rs:26-33).
- **openapi** / **asyncapi**:
  - `qualified_name == key` (structured.rs:111-149).
  - The OpenAPI operations are top-level `operationId` nodes, not grouped under a path, so Meth is 🟡.
- **lean**:
  - `Property` for structure fields and `Constructor` for inductive constructors (lean.rs:431-477).
  - Vis 🟡: a `visibility` string is read from the leading `private`/`protected` keyword (lean.rs:315-340), but no `Vis` enum is set. This is unverified in output.
  - `open X in def` is not extracted (doc fixture `iso/openin`).

## Parity gaps

The gaps are ranked by impact: how many call edges or dead-code verdicts they cost, and how common the shape is in that language's ecosystem. Effort is a rough guess: S = one plugin arm plus tests; M = a new extraction path or a QN change confined to one plugin; L = a cross-cutting change in QN or id formation, or new resolver support.

1. **Class-field / property-held functions are not callables in JS/TS, while named `const` arrows and Rust named closures are.**
   - Gap: `handler = () => { this.go() }` in a class body (the dominant React and Angular handler idiom) has no node, and its calls are `no-enclosing-callable`.
   - Compare: `try_record_named_fn` already names `const f = () =>` (javascript.rs:472-517), and Python's `named_lambda` does the same.
   - Evidence: **fx-b** (`B.go` only, no `B.handler`); typescript doc §02 "Class fields holding functions"; javascript doc fixture f1.
   - Effort: S–M (add a `field_definition`/`public_field_definition` arm that reuses `try_record_named_fn`).
2. **Inline route-handler synthesis exists only in JS/TS, Go, Ruby, Perl, Elixir and Clojure.**
   - Gap: PHP (Laravel `Route::get('/x', function(){})`), C# minimal APIs (`app.MapGet("/", () => …)`), Kotlin (Ktor `get("/x") { }`), Swift (Vapor), Dart (shelf) and Lua all have a registrar-style rule, or a documented gap, but get no `handler_at_` node.
   - Evidence: `grep -ln handler_at_ plugins/*.rs` returns six files; **fx-e** (C# `MapGet` lambda gives no node); php doc §02 fixture p2; the `aspnet-minimal` registrars at rules.rs:1178 and `laravel` at rules.rs:1035 (rules-part-1.md); the `ktor` gap at rules.rs:2346 ("no named callable for a route to attach to").
   - Effort: M per language (port `extract_inline_handler` from javascript.rs:1268-1340 / go.rs:234-271).
3. **Abstract and interface members are not tagged `abstract` in Java, Kotlin, C#, Go, PHP, Groovy, Solidity, Objective-C, C++, Fortran and Elixir, while TS, Swift, Scala and Dart are tagged.**
   - Gap: the duck-typed fan-out narrowing ("drop declarations when a concrete candidate survives") therefore never fires for them.
   - Evidence: `grep '"abstract"' plugins/*.rs` finds only scala.rs:196, dart.rs:119, swift.rs:209 and javascript.rs:370; `cross_file.rs:1152-1190`; go doc §02 fixture f6 (`s.Area()` fans out to `lib.Shape.Area` as well); kotlin doc fixture f2; solidity doc fixture f2.
   - Effort: S each. Bodiless detection already exists for most of them, e.g. `has_body` in java, the interface body in go.rs:405-456 and the pure-virtual check in cpp.rs.
4. **The module, package or namespace is missing from the QN in TS/JS, Scala, C# (file-scoped), PHP, F#, OCaml, Swift, Dart, Zig, Lua, VHDL, Verilog and Starlark, while Java, Kotlin, Groovy, Python, Rust, Go, Erlang, Elixir, Haskell, Julia, Lean, Proto and Smithy have it.**
   - Gap: same-named members of different modules share one QN, which drives `ambiguous-in-file` and fan-out, and makes mermaid labels indistinguishable.
   - The worst cases are bugs, not design:
     - OCaml never sets the module (ocaml.rs:127-137; **fx-a** `mfun`).
     - The C# file-scoped namespace (csharp.rs:157-159; **fx-a**).
     - The TS `namespace` is not a scope (**fx-b**).
   - Effort: S for the OCaml and C# bugs; L for TS/JS/Scala/PHP, because resolvers rely on bare QNs (`cross_file.rs:5605-5608`, `PhpIndex`).
5. **Nested functions lose their enclosing function in the QN, or get the wrong owner, in every language except Python.**
   - Rust is the worst case: a nested `fn` becomes a *method* of the impl (**fx-b** `crate::H::nested_inner`, kind method).
   - Kotlin, Swift, C#, Go, TS/JS, Scala, Julia, Haskell, F#, Lua, OCaml, Starlark, CMake, Nix, Bash, R and HCL flatten nested functions. Python qualifies them as `module.outer.inner` (python.rs:246-262).
   - Evidence: the per-language doc §02 notes; go doc fixture f10 (two `t.handler` nodes); swift doc fixture f12 (two `inner`).
   - Effort: M (push the function on the scope stack; check the lexical-narrowing code paths in `intra_file` that compare QNs).
6. **Properties and accessors: Swift, Kotlin and Objective-C have no accessor nodes, while C#, Python, JS/TS, Dart, F# and Lean do.**
   - Gaps:
     - Swift computed properties, subscripts and `deinit`, including SwiftUI `var body` (**fx-b**; `swiftui` rule gap).
     - Kotlin `get() =` accessors (kotlin doc fixture f8).
     - Objective-C `@property` and Ruby `attr_accessor` (**fx-c**).
     - C# expression-bodied properties (`int P => X()`; csharp doc fixture f1 L20) and accessors named only `get`/`set` (csharp.rs:230-240).
     - Dart top-level and abstract getters (dart.rs:109-111).
   - Effort: M (Swift/Kotlin new arms; C# rename to `Prop.get`, S).
7. **Operator definitions: C# extracts none, while C++, Swift, Dart, Scala, Kotlin, Ruby and Python do.**
   - C# `operator_declaration` and `conversion_operator_declaration` are not matched (**fx-a**).
   - C++ lacks conversion operators (**fx-a**; cpp.rs:412 `operator_cast` arm unreachable).
   - Haskell, OCaml, F#, Julia and VHDL drop operator definitions; Elixir misnames them (`a`).
   - Effort: S for C#, VHDL (accept `operator_symbol`) and OCaml/F# (`parenthesized_operator`); M for Haskell and Julia.
8. **Constructors:**
   - Groovy extracts none, while Java does (**fx-a**; no `constructor_declaration` arm).
   - Dart drops the bodiless `Square(this.side);` and its constructors carry kind `method` (dart.rs:109-111, 151).
   - PHP `__construct` is kind `method` (**fx-c**), while Python `__init__` is `Constructor`.
   - F#, the Scala primary constructor, the SV `new`, Haskell data constructors and Solidity `fallback`/`receive` produce no node.
   - Kotlin interfaces get a spurious constructor (**fx-a**).
   - Effort: S each (Groovy and PHP S; Dart S–M).
9. **Visibility is not recorded in Swift, Ruby, PHP, Scala, Dart, C, C++, Objective-C, Elixir (`defp`), Clojure (`defn-`), Zig (`pub`), F#, Groovy, Erlang (`-export`) and Fortran, while Java, Kotlin, C#, TS, Go, Rust, Python and Solidity record it.**
   - Gap: this feeds the dead-code public-API rules (section 12).
   - Evidence: the hard-coded `visibility: String::new()` cited per row; `grep 'vis:'` hits only csharp, java, javascript, kivy, kotlin, python and rust (plus go_vis and the solidity mapping).
   - Effort: S–M per language.
10. **Attributes and annotations are not recorded in Swift (`@objc`, `@IBAction`, `@main`), C (`__attribute__`), Objective-C (`IBAction`), Zig (`export`), Dart (`@override`, `@pragma`), PowerShell (`[CmdletBinding()]`), Perl (`:Path`) and OCaml (`[@@…]`), while Java, Kotlin, C#, Python, TS, Rust, Scala, Groovy, F#, PHP and Lean record them.**
    - Gap: framework rules keyed on attributes cannot match.
    - Evidence: `attributes: Vec::new()` in swift.rs:114/179, c.rs:216/247, objc.rs:102/209, zig.rs:201, dart.rs:267, powershell.rs:157/216.
    - Effort: S each (the shared `attrs::collect` exists).
11. **Over-extraction and spurious nodes:**
    - Haskell type variables and do-binds (**fx-b** `Data.Thing.a`).
    - OCaml plain values and `let _ =` (**fx-a** `value`).
    - Clojure metadata kept inside names (**fx-d**), so calls miss.
    - Go multi-assignment `a, b` (**fx-c**).
    - Perl forward declarations (**fx-c**).
    - Effort: S each.
12. **Static/class method variant: C#, C++, PHP, Dart, F#, Swift `class func` and Ruby `class << self` lack it, while Java, Python, JS/TS, Groovy, PowerShell and Objective-C have it.**
    - Evidence: the `StaticMethod` grep counts (none in csharp.rs, cpp.rs, php.rs, dart.rs or fsharp.rs); **fx-b** `Sq.klass`; **fx-e** `Sq::build`.
    - Effort: S.
13. **Macro and generated definitions:**
    - Objective-C does not extract fn-like `#define`, while C and C++ do through `c_macro.rs` (objc doc §02, fixture f2; objc.rs has no `preproc_function_def` arm).
    - Erlang `-define` function generators and Julia `macro` definitions are absent, while Rust, C, C++, Elixir and Clojure macros are nodes.
    - Ruby `define_method`, Kotlin data-class members and Java Lombok/record accessors are absent.
    - Effort: S (ObjC: reuse `c_macro`), S (Julia: read the `signature` child), L (Ruby/Kotlin/Java synthesis).
14. **Overload grouping: VHDL, PowerShell and Python `@overload` produce separate same-QN nodes but are not in `binds_same_name_set`, while Java, C++, C#, Kotlin, Scala, Swift, Groovy, F#, Solidity, TS and Julia are (`intra_file.rs:835-850`).**
    - Clojure multi-arity `defn` is a single node, and `defmethod` shares the `defmulti` QN.
    - Effort: S.
15. **Interface and declaration-only members missing entirely:**
    - Clojure `defprotocol` members (**fx-d**).
    - Haskell class method signatures (**fx-b**).
    - OCaml `.mli`/`module type` `val`s, F# `abstract member` and Erlang `-callback`.
    - Compare: Java, Swift and TS extract them.
    - Effort: M.
16. **Top-level and initializer code is a callable in no language.**
    - Kotlin is the only partial case, because `init {}` is covered by the class-as-constructor span.
    - Affected: Java `static {}`, C# top-level statements, Swift `main.swift`, Zig `test`, Lean `example`, Groovy/Gradle scripts, PowerShell, Bash, R, CMake and OCaml `let () =`. Their calls are `no-enclosing-callable` (per-language docs).
    - Effort: L (needs a synthetic `<module>`/`<init>` node and a dead-code rooting change).
17. **Descriptor languages:**
    - Proto nested `message`/`enum` declarations are dropped (proto.rs:89-96), while Smithy has no nesting concept and OpenAPI drops `callbacks` operations.
    - GraphQL `extend type` and fields produce no node, while Smithy/Proto members are at least reference sources.
    - OpenAPI/AsyncAPI QNs carry no section, so they collide; Smithy uses `ns#Name`.
    - Effort: S–M.
18. **Owner-less members of secondary scopes:**
    - Dart enum and extension members (**fx-b** `paint`).
    - Scala 3 `enum`/`given`/`extension`.
    - F# record/union/extension members (**fx-d** `Double`).
    - Elixir `defimpl`, Objective-C protocols, the Ruby compact-name doubling, and Java/Scala/Groovy/PHP anonymous classes collapsing into the enclosing class.
    - Compare: Java and C# own enum members correctly.
    - Effort: S–M.

## Doc corrections

No language doc was found to be wrong on a fact checked here: every claim re-run on **fx-a** … **fx-e** reproduced, and every `file:line` spot-checked still holds. The spot-checks were:

- csharp.rs:199-201
- cpp.rs:393-399 and 411-416
- elixir.rs:324-328
- clojure.rs:216-218
- fsharp.rs:138-141
- ocaml.rs:127-137
- perl.rs:94-96
- vhdl.rs:220-226
- hcl.rs:101
- kotlin.rs:327-333
- groovy.rs:129
- intra_file.rs:858-859

Clarifications and omissions:

- **dart.md §02**: "Constructor with a body … Recorded as `K::K`" does not say that the node's variant is `InherentMethod` (kind `method`). dart.rs has no `DefVariant::Constructor` (grep), and the constructor arm records `InherentMethod` (dart.rs:151). The same applies to "Static method … Recorded as `Square::build`": the kind is `method`, and no static variant exists (**fx-e**).
- **groovy.md §02** claims that `closure_expression` names a non-existent kind. This is confirmed against `tree-sitter-groovy-0.1.2/src/node-types.json`, which has `closure` and no `closure_expression`. It is not a correction, but the evidence is recorded here because the doc cites only the plugin line.
- **julia.md §02** claims that `macro_definition` has no `name` field. This is confirmed: the 0.23.1 `node-types.json` lists no fields, and its children are `_expression`, `assignment`, `open_tuple` and `signature`.
- **lean.md §02** does not mention visibility. lean.rs:315-340 reads `private`/`protected` into `DefRecord::visibility`, so Lean is the only "minor" plugin that records a visibility string. This is unverified in JSON output (`vis` was not checked).
- **Code comment, not a doc**: the comment on `binds_same_name_set` (`intra_file.rs:827-830`) lists "TypeScript overload signatures" as same-QN sets. typescript.md §02 (fixture f1) and the plugin (javascript.rs:212-221, which records `method_signature` only inside `interface_body`) show that overload signatures are never extracted. For TypeScript, the set therefore groups only redefinitions and the single implementation.
- **CLAUDE.md** names only TS/Swift/Scala/Dart as tagging bodiless declarations `abstract`, which matches the code. Several language docs (java.md, kotlin.md, csharp.md, go.md, solidity.md, objc.md) correctly describe their members as untagged. java.md §02 does not state it explicitly, but its interface/abstract bullets imply it.
