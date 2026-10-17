# julia

The Julia plugin (`crates/cgg-lang/src/plugins/julia.rs`, 442 lines, `JuliaPlugin`, id `"julia"`) is a single hand-written tree walker over the `tree-sitter-julia` crate, version 0.23.1 (`Cargo.toml:109`, `tree-sitter-julia = "0.23"`; bound at `julia.rs:21-23` as `tree_sitter_julia::LANGUAGE`). No grammar is vendored. It does not override `resolver_kind`, `signals()`, `max_line_bytes` or `ts_language_for`, so it takes the trait defaults (`crates/cgg-lang/src/lib.rs:295-340`). That means no dead-code signals are declared. It ignores `ExtractCtx` (`_ctx`, `julia.rs:27`). The plugin records only definitions (all `DefVariant::FreeFunction`), call references (bare name text, no receiver, no arity) and `using`/`import` records. It records no locals, classes/bases, packages, attributes, visibility, value refs, name mentions or dyn uses. Resolution is the generic intra-file and cross-file pipeline. Julia has a few explicit memberships: `lexically_scoped` (`crates/cgg-resolve/src/intra_file.rs:119-135`), `binds_same_name_set` (multiple-dispatch methods as one callee, `intra_file.rs:830-852`), and the "dotted module, unqualified import" arm of the `import` handler (`crates/cgg-resolve/src/cross_file.rs:2063-2077`).

Fixtures for this catalogue are in `<scratch>/parity/julia/` (p1 to p6, run with `target/release/cgg` 0.11.0).

## 01. Detection & parsing

Supported:

- **Extension `.jl`.** Declared at `julia.rs:15-17`. Fixture p1: `detected_via: "extension:.jl"` in the audit.
- **Shebang `julia`.** Declared at `julia.rs:18-20`. Fixture p2/script (`#!/usr/bin/env julia`, no extension): `detected_via: "shebang:julia"`.
- **Grammar.** One grammar, `tree_sitter_julia::LANGUAGE` (`julia.rs:21-23`). There is no per-file dialect choice (`ts_language_for` default, `lib.rs:322`).
- **Generic skip gates.** These apply as they do to every language: tree depth over `MAX_TREE_DEPTH` = 4000 gives `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155`), and minified or long-line files are handled by the shared gate (`lib.rs:157`, `SkipReason::LongLine` at `lib.rs:439`). Julia sets no `max_line_bytes`, so the default is `None` (`lib.rs:310-312`).
- **Malformed input does not panic.** Test `malformed_source_does_not_panic` (`julia.rs:417-420`); `an_empty_file_yields_nothing_and_does_not_panic` (`julia.rs:410-414`).

Not supported:

- **Content detection.** There is none: no header or content sniffing for Julia in `detect.rs` (grep finds no `julia` in `crates/cgg-lang/src/detect.rs`).
- **Julia-kernel notebooks.** `.ipynb` Julia kernels are out of scope; notebooks are treated as Python (`crates/cgg-lang/src/notebook.rs:11-13`).
- **Re-parse or fallback.** None (contrast C#'s `#if` blanking re-parse). Parse errors are left to tree-sitter error recovery.

## 02. Callables extracted

Every definition is `DefVariant::FreeFunction` (`julia.rs:89,110,120,144`). The qualified name is the enclosing `module` names joined with `.` plus the simple name (`qn`, `julia.rs:55-61`; the module push is at `julia.rs:65-77`). The `signature_hint` is `extract_signature(node text)` (`julia.rs:213`). Visibility is always empty and attributes are always empty (`julia.rs:214-215`).

Supported:

- **Long-form function** `function greet(name) … end`. The name is the signature text before the first `(` (`julia.rs:78-92`). Test `function_definition_captured` (`julia.rs:335-339`).
- **Short-form method** `double(x) = 2x`. It parses as an `assignment` whose left side is a `call_expression` with an `identifier` head (`julia.rs:136-159`). Test `short_form_assignment_definition_captured` (`julia.rs:342-351`).
- **Short form under `where` / return type.** `f(::Type{T}, x) where {T} = x` and `h(x)::Int = x` are both definitions (`short_form_head`, `julia.rs:280-292`). Test `parametric_calls_and_short_form_methods` (`julia.rs:426-441`). Near-miss: `x::Int = g(1)` is not a definition (same test).
- **Long form with `where`.** `function area(x::T) where {T<:Number}` is a definition (fixture p1 `Geo.area:18`). Its `signature_hint` is cut to `"function area(x::T) where"` (fixture p1 dead-code JSON).
- **Multiple-dispatch methods.** `area(c::Circle) = 1; area(s::Square) = 2` gives two separate definitions under one qualified name. Test `multiple_dispatch_methods_are_separate_definitions` (`julia.rs:392-407`); fixture p1 has three `Geo.area` nodes.
- **Module scoping** `module M … end`. Definitions get `M.` (`julia.rs:65-77`). Test `a_module_qualifies_its_definitions`. Nested modules are chained: fixture p4 `Outer.Inner.deep`.
- **Outer constructors** `Circle(r::Int) = Circle(Float64(r))`. Recorded as `Geo.Circle` (fixture p1).
- **Inner constructors inside `struct`.** `P(x) = new(x)` and `function P() new(0) end` are recorded as `Outer.P` twice. The struct is not a scope, so the name is not `Outer.P.P` (fixture p4).
- **Macro-prefixed definitions.** `@inline fast(x) = x` and `@generated function gg(x) … end` are recorded (fixture p4 `Outer.fast`, `Outer.gg`). The macrocall's children are walked.
- **Keyword-argument methods** `kw(x; k=1) = x + k`. Recorded (fixture p4).
- **Nested named functions.** `function outer() function inner() … end end` and `loc() = 1` inside a function are recorded. The qualified name carries only the module, not the enclosing function (`Geo.inner`, `Outer.loc` in fixtures p1/p4), because `scope` only holds modules.
- **Qualified-name long form** `function Base.show(io::IO, c::Circle)`. Recorded with simple name `Base.show` and qualified name `Geo.Base.show` (fixture p1). The module prefix is prepended to the already dotted name.
- **`__init__`.** An ordinary function, recorded as such (fixtures p1, p5).

Not supported or partial:

- **Macro definitions** `macro twice(ex) … end` are NOT extracted. The plugin looks for a `name` field (`julia.rs:116-125`), but `macro_definition` in tree-sitter-julia 0.23.1 has no fields (its `node-types.json` lists only a `signature` child). The arm also does not use `walk_signature`, so the head `twice(ex)` is recorded as a call to `twice` (fixture p1: `external_calls … "twice"` at the `macro` line 30).
- **`short_function_definition`** (`julia.rs:106-115`) is a node kind that does not exist in grammar 0.23.1 (absent from `node-types.json`). The arm is dead code; the short form is handled through `assignment`.
- **Short-form methods with a qualified or operator name are NOT definitions.** `Base.length(c::Circle) = 1` (head is a `field_expression`) and `Base.:+(a::P, b::P) = …` fail the `identifier` check at `julia.rs:140`. They are recorded as calls named `Base.length` / `Base.:+`, screened as stdlib (fixtures p1 line 26, p4 line 13).
- **Operator method definitions** `+(a::P) = a` are not extracted; the head is recorded as a call to `+` (fixture p4 line 14, external).
- **Callable-struct (functor) methods** `function (c::Circle)(x) … end` are NOT extracted. The signature text starts with `(`, so the name before the paren is empty (`julia.rs:86-90`). Calls in the body have no caller (fixture p1 line 28, `src: null`). The `flux` rule gap (`rules.rs:6399`) states the same.
- **Zero-method generic functions** `function gen end` are not extracted: there is no `(` in the signature (fixture p4, `gen()` is external).
- **Anonymous functions and lambdas** (`x -> x * 2`, `function(z) z end`, `do` blocks) are not callables. Calls inside them are attributed to the enclosing named function (fixture p1: `helper` inside `map(x -> helper(x), v)` has `src` `Geo.outer`).
- **Types.** `struct`, `mutable struct`, `abstract type` and `primitive type` are not callables and are not recorded as classes. Default (field) constructors therefore do not exist as nodes (fixture p1: `Circle(1.0)` binds only to the outer constructor).
- **Assigned aliases** `const alias = f1` record nothing (fixture p4).
- **Owners.** No owner or `InherentMethod` variant is ever produced: Julia methods belong to generic functions, not types.

## 03. Call-site forms

`record_call` (`julia.rs:255-274`) records the text of a `call_expression`'s first child as `RefRecord::name`. `receiver_hint` is always empty and no other `RefRecord` field is set.

Supported:

- **Bare call** `inner(1)`. Test `calls_in_a_body_are_references` (`julia.rs:382-389`).
- **Explicit type parameters** `Foo{Int}(x)`. The `parametrized_type_expression` is stripped to its head, so the call is recorded as `Foo` (`julia.rs:257-262`). Test `parametric_calls_and_short_form_methods`.
- **Signature heads are not calls.** A function signature is not recorded as a call to itself, but default values in it are walked (`walk_signature`, `julia.rs:170-185`). Test `a_function_signature_is_not_a_call_to_itself` (`julia.rs:325-332`); CHANGELOG A10 "every `function f()` drew a `high` self-edge".
- **Short-form left sides are not calls.** Only the right-hand side is walked (`julia.rs:145-154`).
- **Calls inside macro invocations** `@twice helper(1)` and `@test Geo.area(…)`. The macrocall's children are walked, so the inner `call_expression` is recorded (fixture p1 line 45, `helper`).
- **`do`-block calls** `do_stuff(v) do y … end`. Recorded as a call to `do_stuff` (fixture p1 line 56).
- **Keyword-argument calls** `kw(1; k=2)`. Recorded and bound (fixture p4).
- **Prefix-operator call form** `-(a)`. Recorded as a call named `-` (fixture p1 line 60, external).

Partial:

- **Qualified or path calls** `Utils.tool(3)`, `A.fa(3)`, `Inner.deep(3)`, `Geo.helper(1)`. Recorded as the whole dotted text, with no receiver split and no `path_receiver`. Nothing binds them (fixtures p1/p2/p4: `no-candidate-in-file`; `Inner.deep(3)` stays unresolved while the bare `deep(4)` binds). Calls into `Base.`/`LinearAlgebra.` are screened as stdlib (fixture p1, `LinearAlgebra.dot`).
- **Field-call syntax** `obj.method(6)`. Recorded under the name `obj.method`, screened external (fixture p2).
- **Calling a call result** `getfield(Geo, :area)(c)`. Recorded under the full text `getfield(Geo, :area)` as an external name, plus the inner `getfield` (fixture p1 line 54).
- **Constructor calls** `Circle(1.0)`. Bound to a user-written outer or inner constructor with that name, if any; nothing else models construction.

Not supported:

- **Broadcast calls** `inner.(v)`. `broadcast_call_expression` is not handled (fixture p1 line 47: no reference to `inner`).
- **Infix operator calls** `p + p`, `a + b`, `v .|> inner`. `binary_expression` is not recorded (fixtures p1/p4).
- **Pipe** `v |> sum`. Not recorded as a call of `sum`.
- **Functions passed as values** `map(inner, v)`, `route("/", handler)`. Not recorded: there are no value refs (`ExtractCtx` ignored, `julia.rs:27`). Fixture p1: `Geo.inner` has no inbound edge and is reported dead.
- **Calls through a local closure** `g = x -> …; g(2)`. Recorded as `g`, external (fixture p1 line 53).
- **Arity, paren-less calls, registrar arguments, string targets.** None recorded (`RefRecord` built with defaults, `julia.rs:265-271`).
- **Macro invocations as references** `@twice …`. Not recorded as a reference to the macro (macros are not definitions anyway; see §02).

## 04. Imports & module resolution

`record_import` (`julia.rs:220-253`) records one `ImportRecord` per direct `identifier`/`dotted_identifier`/`scoped_identifier` child of a `using_statement` or `import_statement`. The kind is `"using"` when the statement text starts with `using`, otherwise `"import"`. The alias is the last dotted segment.

Supported:

- **`import A`.** Kind `import` goes through the `"import"` arm, whose Julia branch (`cross_file.rs:2063-2077`) pushes the path to `unqualified_prefixes` and maps the last segment into `module_aliases`. Fixture `q` (`import A`, then `fa(1)` in another file): `main -> A.fa high cross-file:imports`.
- **Step 1c scoped lookup.** Imported module paths become lowercase path fragments matched against file paths (`cross_file.rs:2192-2218`, `ScopedSimple::get_capped` at `cross_file.rs:139-163`). Fixture p3: `Mylib.foo` and `bar` bound `medium cross-file:imports`.
- **Step 1d global fallback.** When the file has any import record and the bare name has 3 or fewer workspace candidates and is not in the stdlib list, the call binds by name (`cross_file.rs:6018-6060`). This is why fixture p3 bound `foo` even though `using Mylib` itself contributes nothing.
- **Test.** `using_and_import_keep_their_own_kinds` (`julia.rs:363-379`) checks the extracted records only.

Not supported or partial:

- **`using A` contributes nothing to resolution.** Kind `"using"` matches no arm: the only `using` arm is `"using" if lang == "csharp"` (`cross_file.rs:1767`), and the rest fall to `_ => {}` (`cross_file.rs:2160`). Fixture `q`: `using A` + `fa(1)` gives no edge (`candidates-in-other-files`). The idiomatic Julia import form is therefore inert unless some other import exists in the file.
- **`import A: fa`, `using LinearAlgebra: norm`.** These are `selected_import` children, which `record_import` does not descend into, so no record is made (fixture `q`: no edge; fixture p6 `import Base: show` did not even trigger framework detection).
- **Relative imports** `using .A` / `using ..A` (`import_path` node) record nothing (fixture `q`).
- **Aliased imports** `import A.fa as alias_fa` (`import_alias` node) record nothing; `alias_fa(4)` is external (fixture p2).
- **Multiple modules** `using A, B`. Recorded as two `using` records, still inert (fixture `q`).
- **`include("util.jl")`.** Not an import. It is recorded as a call to `include`, screened stdlib (fixture p1 line 5). Functions in included files do not bind from the including file without some import (fixture p1: `helper` gives `candidates-in-other-files`).
- **`export`.** Not recorded; it plays no part in resolution or in dead-code roots (fixture p1: an exported `area` is not a root).
- **Module-qualified calls never bind.** `A.fa(3)` (see §03). There is no `ModuleIndex` / `not-in-imported-module` handling for Julia (`modules.rs` has no `julia`).
- **No `FileFacts::packages`, no manifest reading** (`Project.toml` is not read; no `julia` in `plugins/manifest.rs`).
- **No `heads.rs` external-module refusal.** Julia is not among the `WorkspaceVocab` languages (CLAUDE.md lists Python/JS/TS/Go/Java/Kotlin/C#/C++; grep finds no `julia` in `heads.rs`). Names not found are screened stdlib or external by name list only.
- **Bare names cross module boundaries.** Fixture p4: bare `deep(4)` in module `Outer` binds `high` to `Outer.Inner.deep` with no `using .Inner`. Intra-file lookup ignores module membership.

## 05. Type inference & receiver typing

Not applicable in practice: the plugin records no parameter types, local types, return types, field types or receivers. `receiver_hint` is always `String::new()` (`julia.rs:267`). There is no `julia` in `type_hints.rs`, `closure_types.rs` or `receiver.rs` (grep).

- `c = Circle(1.0); area(c)` does not narrow `area` by argument type. All three `Geo.area` methods bind at medium (fixture p1).
- Multiple dispatch on argument types is the Julia analogue of receiver typing, and it is not modelled. There are no argument-type records (`RefRecord::arg_type` is unused).
- There are no `declared_type` wrappers (`Union{Nothing,T}`, `Nullable`), no casts or constructions (`CAST_INTRINSICS`/`CONSTRUCTING_METHODS` have no Julia entries; `stdlib.rs` mentions Julia only at lines 206/247), no implicit receivers and no weak receiver types.

## 06. Inheritance, dispatch & overloading

Supported:

- **Same-file multiple-dispatch methods are one callee.** `binds_same_name_set` includes `"julia"` (`intra_file.rs:835-852`). A bare call to a name with several same-file methods binds every method at `medium` (fixture p1: `Geo.outer -> Geo.area` ×3 medium; fixture p4: `caller -> Outer.P` ×2 medium, mermaid `-->|?|`). CHANGELOG: "Overload sets past the cap now bind in … Julia (+308)".

Not supported or partial:

- **No method selection.** There is no arity or argument-type narrowing: Julia is not in `OVERLOAD_ARITY_LANGS` (`crates/cgg-resolve/src/type_hints.rs:1583`, `["java","scala","cpp"]`), and no `arity` is recorded.
- **Calls from code with no caller.** A call to such a name from a callable-struct body (no enclosing callable) is recorded `ambiguous-in-file` (fixture p1 line 28, `file_local: 3`).
- **No inheritance.** `abstract type Shape end` and `struct Circle <: Shape` are not recorded (no `ClassDecl`, no `base_types`). Julia is not in `INHERITANCE_LANGS` (`crates/cgg-resolve/src/dispatch.rs:110-123`), so `inheritance_fanout` never runs.
- **No `--dynamic-dispatch` edges.** No `trait_impl_target` is set; the dead-code capability table reports `dispatch: none` (fixture p1).
- **Methods extending another module's function** (`function Base.show(…)`, `Base.length(x) = …`). Not linked to the generic function; only the long form is extracted, and it is only rooted via `julia-base-dispatch` (§10).
- **Not applicable to Julia:** super/base calls, `member_of_calling_class`, `overrides_below`, extension methods, mixins.

## 07. Scoping & shadowing

Supported:

- **Lexical narrowing of nested definitions.** `lexically_scoped` includes `"julia"` (`intra_file.rs:119-135`). Fixture p4: `a1 -> Outer.loc:23` and `a2 -> Outer.loc:27`, each `high` to its own nested `loc`. CHANGELOG: "Lexical scope for bare calls beyond Python: … Haskell and Julia".

Not supported:

- **No `FileFacts::locals`.** `locals::spec` has no Julia table (`crates/cgg-lang/src/locals.rs:380-394`), so there is no `shadowed-by-local`. Fixture p6: `function user(helper) helper() end` binds `M.user -> M.helper` at `high`, but the parameter shadows it.
- **No positional shadowing** (that is only OCaml, F# and Lua).
- **No conditional-definition tracking** (`DefRecord::conditional` is not set). `if cond; f() = 1; else; f() = 2; end` gives two same-name methods that bind as an overload set (inferred from `binds_same_name_set`; unverified by fixture).
- **Module boundaries are not scopes for bare-name lookup** (see §04, `deep(4)`).

## 08. Macros, preprocessor & codegen

- **Macro definitions are not extracted, and their head is recorded as a call** (§02; fixture p1 line 30, `twice` external).
- **Macro invocations are not expanded.** The invocation is walked for nested calls (§03). Definitions under a macro prefix (`@inline`, `@generated`) are kept as plain definitions (fixture p4).
- **No `macro:<name>` tagging.** Only `rust_macro.rs` and `c_macro.rs` produce it (grep `"macro:` finds only `c_macro.rs`/`rust_macro.rs`).
- **No `name_mentions`.**
- **Generated methods are invisible.** `quote … $(esc(ex)) … end` calls (`esc`) are recorded as stdlib calls from no callable (fixture p1 line 32), and `@eval` loops generating methods are not modelled. The `flux` rule gap names `@layer`/`@functor`-generated methods and `@non_differentiable` rules (`rules.rs:6399`).
- **Not applicable:** preprocessor and `#if`.

## 09. FFI & cross-language links

None. `ffi::link_ffi` has no Julia handling: `ccall`/`@ccall` to C libraries are not linked, and neither are PyCall/PythonCall/jlrs/CxxWrap (grep finds no `julia` in `crates/cgg-resolve/src/ffi.rs`). `ccall((:f, lib), …)` is recorded only as a call named `ccall`. `ccall` is not in `stdlib/julia.txt` (grep `^ccall$` finds nothing), so it lands in the external bucket. That is inferred from the screening behaviour and was not run on a fixture. There are no descriptor links and no markup code-behind.

## 10. Frameworks & entry points

Rules with `language: "julia"` in `crates/cgg-core/src/frameworks/rules.rs`. Detection is by the file's import records (`detect` prefixes); `detect_prefixes.rs:70` synthesises `using {prefix}` for Julia.

- **`julia-module-init`** (`rules.rs:6333-6361`). Kind Lifecycle, `node: true`. Detected by an import of `Base, Core, Pkg, Test, LinearAlgebra, Random, Statistics, Printf, Dates, Logging, Serialization, SparseArrays`. Triggered by `methods: ["__init__"]`. Fixture p1: mints `<framework-entry>::lifecycle::julia-module-init::Geo.__init__` with a `low` `framework_entry` edge to `Geo.__init__`. Fixture p5 (no import): not minted, so a module whose `__init__` sits in a file that imports none of those prefixes gets no root.
- **`julia-test`** (`rules.rs:6363-6385`). Kind Test, detect-only (no attributes, methods or registrars). Detected by `Test, ReTest, TestItems, TestItemRunner, ParallelTestRunner`. Fixture p1 prints "seen, no rules julia-test — … entries NOT enumerated". Gap: `@testset`/`@test` are macro blocks, not callables; test files come via `include`.
- **`flux`** (`rules.rs:6386-6402`). Kind Lifecycle, detect-only. Detected by `Flux, NNlib, Lux, Optimisers, Zygote`. Gap: callable-struct forward passes are not extracted at all.
- **`genie`** (`rules.rs:6403-6419`). Kind Network, detect-only. Detected by `Genie, Oxygen, Bukdu`. Gap: `route("/p", handler)` and `@get`/`@post` handlers are not linked, because no argument metadata is recorded.
- **`julia-base-dispatch`** (`rules.rs:6420-6464`). Kind Lifecycle, `node: false` (root mark only). Same detect list as `julia-module-init`. Triggered by `methods: ["Base.show","Base.print","Base.display","Base.summary","Base.iterate","Base.length","Base.size","Base.getindex","Base.setindex!","Base.eltype","Base.similar","Base.copy","Base.hash","Base.isequal","Base.convert"]`.
  - Fixture p1: `Geo.Base.show` is a root, `framework:rule … julia-base-dispatch:base-type`.
  - Fixture p6: with only `import Base: show` (a `selected_import`, not recorded), the rule is not detected and `M.Base.iterate` is reported dead. `function show(…)` after `import Base: show` does not match the `Base.`-qualified method names.
  - Short-form `Base.length(x) = …` is never a definition, so it cannot be rooted (§02).
- **Benchmark coverage.** `scripts/benchmark.sh:223-224,410,417` lists `julia-flux`, `julia-genie`, `app-alphapem-genie` and `app-crnn-flux` with these rules. The comment at `benchmark.sh:460` reads "julia-base-dispatch 35 entries julia-flux"; at `:466`, "julia-module-init detected, enumerated nothing julia-flux".
- **None.** `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, registrar verbs and visibility entries (grep finds no Julia rows; the plugin records no attributes, which these require).

## 11. Test harness entries

- **`julia-test` is detect-only** (§10). No harness node is minted and no `@testset`/`@test`/`@testitem` case is enumerated (fixture p1).
- **Name-convention test roots work generically.** `is_test_name` falls to `_ => name.starts_with("test_") || name.starts_with("Test")` (`crates/cgg-resolve/src/deadcode/roots.rs:975-989`). Fixture p1: `test_helper` is a `test-entry` root (`builtin:test-name`).
- **Test files are classified by directory.** Fixture p1: `test/runtests.jl` has `test_role: "directory"` in the audit.
- **Calls inside `@testset … begin … end` at top level have no caller.** The fixture p1 runtests site `Geo.area` has `src: null`, and is also an unbound qualified call.
- **Not applicable:** `TEST_CLASS_MEMBERS_ARE_ROOTS` (no classes).

## 12. Dead-code analysis

- **Capability class `degraded`.** The fixture p1 dead-code JSON has `capabilities[julia]`: `class: degraded, visibility: none, attributes: none, value_references: none, dispatch: none, exports: none, test_tagging: none, max_confidence: medium`. `signals()` is not overridden, so every declared bit is false (`crates/cgg/src/lib.rs:1921-1940`). `caps::measure` classifies a language lacking visibility and attributes as Degraded (`crates/cgg-resolve/src/deadcode/caps.rs:160-170`). No Julia finding can reach `high`.
- **Roots seen:** `framework:rule` (julia-base-dispatch), `framework:entry-node` (julia-module-init), `builtin:test-name` (fixture p1 `roots`).
- **No public or exported roots.** `export` is not recorded and visibility is always empty (`julia.rs:214`).
- **Evidence kinds.** Fixture p1 findings carry `language-lacks-attributes` and `language-lacks-dispatch-model`, plus `ambiguous-site-in-same-file`, which lowers the `area` methods to `low`.
- **False findings.** Functions passed as values (`map(inner, v)` gives `Geo.inner` "never-referenced") are reported at medium (fixture p1). So are functions reached only through `using` from another file (`helper`, `Utils.tool`, `do_stuff`).
- **Not applicable to Julia:** `implements-external-contract`, `macro-generated` and reflection caps (none produced).
- **No unreachable-code detection** (`unreachable: false`).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/julia.txt`, 248 lines.** It holds names like `abs`, `map`, `print`, `include`, `Float64`, `Base`, `LinearAlgebra` and macro names (`@assert`, `@inline`, `@eval` …). It is loaded at `crates/cgg-core/src/stdlib.rs:206,247`. CHANGELOG (stdlib audit) lists it as one of eight lists "seeded from language references only", not tuned against corpus noise.
- **Dotted calls with a stdlib head screen as stdlib.** Fixture p1: `LinearAlgebra.dot`, `Base.length` and `Base.:+` land in `stdlib_calls`. Under `--include-stdlib` they become `<stdlib>::LinearAlgebra.dot`, `<stdlib>::Float64`, `<stdlib>::map` (fixture p1 `s.json`).
- **External bucket.** Unknown names (`norm` from `using LinearAlgebra: norm`, `eval`, `-`) go to the external bucket, and `--include-external` mints `<external>::…` leaves (fixture p1).
- **None:** `rust_types`-like wrapper tables, `CAST_INTRINSICS`/`CONSTRUCTING_METHODS`/`MAYBE_TYPE_WRAPPERS`/`UNION_TYPE_CONSTRUCTORS`/`NULL_TYPE_MEMBERS` Julia entries (no `julia` in `stdlib.rs` beyond lines 206/247), and implicit-package tables (`Base`/`Core` are only in the name list).

## 14. Confidence & certainty

- **`high` from intra-file single candidates.** Fixture p4: `caller -> fast/gg/kw/f1/f2/Inner.deep` high; `a1 -> loc` high. This includes wrong highs: a parameter-shadowed name (fixture p6 `user -> helper` high) and a bare call into a nested module never brought in with `using` (fixture p4 `deep(4)`). A constructor calling its own name also binds high to itself (fixture p1 `Geo.Circle -> Geo.Circle` high, `Circle(r::Int) = Circle(Float64(r))`): the intended target, the default struct constructor, is not a node.
- **`high` from `import A` + bare call to a unique `A.f`.** Fixture `q`: `main -> A.fa high cross-file:imports`.
- **`medium`:** multiple-dispatch sets (every method), step 1c/1d by-name cross-file matches (fixture p3). The framework entry edge is `low`.
- **No `CertainCtx` typed-receiver or `OwnerFacts` path.** No receivers.
- **Mermaid `?` marker.** Fixture p4 renders `N12 -->|?| N1` for the medium `Outer.P` overload edges.
- **Historical numbers.** CHANGELOG records 997 high and 1,059 medium signature self-edges removed (A10), and WS9 "Julia high 1 / 27, medium 0 / 71".

## 15. Dynamic use & reflection

None. The Julia plugin does not call `dynuse::extract` (only java, ruby, go, javascript, php, typescript, csharp and python plugins do; grep `dynuse::extract`), and `REFLECTIVE` has no Julia row (`crates/cgg-lang/src/plugins/dynuse.rs:24-…`).

- `getfield(Geo, :area)(c)`, `eval(:(helper(2)))`, `@eval`, `invokelatest`, `Symbol("f")` and `getproperty` lookups by literal name are recorded only as ordinary calls (`getfield` stdlib, `eval` external; fixture p1). They neither cap dead-code findings nor create edges.
- Module loads (`Base.require`, `include(path)` with a computed path) are not module-load screens.

## 16. Known gaps & limitations

- **`using M` is inert in cross-file resolution.** The kind `"using"` reaches no Julia arm (`cross_file.rs:1767,2160`); only `import M` feeds `unqualified_prefixes` (`cross_file.rs:2063-2077`). Fixture `q`.
- **`selected_import` (`import A: f`, `using A: f`), `import_path` (`using .A`) and `import_alias` (`import A.f as g`) are never recorded** (`julia.rs:228-231` only accepts `identifier`/`dotted_identifier`/`scoped_identifier`). This also blocks framework detection (fixture p6).
- **Module-qualified calls (`Mod.f(x)`) never bind**, not even within one file (fixtures p1, p2, p4). README benchmark: `Flux.jl | julia | 490 | 1,001 | 0%` cross-file (`README.md:1334`).
- **`include("file.jl")` is not followed.** That is the main way Julia packages assemble a module from files (fixture p1); the `julia-test` gap (`rules.rs:6382`) says the same.
- **Macro definitions are not extracted** (no `name` field in grammar 0.23.1), and the macro head is recorded as a self-named external call (`julia.rs:116-125`, fixture p1).
- **`short_function_definition` arm is dead.** That node kind is not in grammar 0.23.1 (`julia.rs:106-115`).
- **Callable-struct methods `function (m::T)(x)` are not extracted** (`rules.rs:6399`, fixture p1).
- **Short-form definitions with a qualified or operator name are recorded as calls, not definitions:** `Base.length(x) = …`, `Base.:+(a,b) = …`, `+(a) = a` (fixtures p1, p4).
- **No zero-method `function f end`, no lambda / anonymous / `do` callables, no struct or abstract-type records.**
- **No locals**, so parameters that shadow globals bind high to the wrong target (fixture p6). No receiver, argument-type or arity data, so multiple dispatch always fans out to every method.
- **No value references, broadcast calls (`f.(x)`), infix operator calls or pipes (`x |> f`)** (fixture p1).
- **Bare-name lookup ignores module boundaries** (fixture p4 `deep(4)` high).
- **Nested function qualified names omit the enclosing function** (`Geo.inner`), because `scope` only tracks modules (`julia.rs:47,65-77`).
- **Dead-code class is `degraded`:** no visibility or `export`, no attributes, so `max_confidence` is medium.
- **Framework rules `julia-test`, `flux` and `genie` are detect-only.** `julia-module-init` and `julia-base-dispatch` need one of the listed `using` imports in the same file (fixture p5).
- **`stdlib/julia.txt` is seeded from language references, not corpus-tuned** (CHANGELOG stdlib audit).
- **No Julia-kernel notebooks** (`notebook.rs:11-13`).
- **Tests.** There are no `#[ignore]` Julia tests. Coverage is the 10 inline tests in `julia.rs:294-442` plus the detection-only `crates/cgg/tests/detect_prefixes.rs:70`. No resolver-level test exercises Julia cross-file resolution (grep finds no `julia` in `crates/cgg-resolve` tests).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | full | `.jl` + `julia` shebang; single grammar; generic skip gates; no content sniffing |
| 02 Callables extracted | partial | long and short form, `where`/return-typed, dispatch methods, modules; no macros, functors, operator or `Base.f(x)=` short forms, lambdas or types |
| 03 Call-site forms | partial | bare, `Foo{T}()`, do-block and macro-arg calls; qualified calls kept as one dotted name; no broadcast, infix, pipe or value refs |
| 04 Imports & module resolution | minimal | only `import M` feeds resolution; `using`, `import M: f`, `using .M`, aliases and `include` are ignored |
| 05 Type inference & receiver typing | none | no receivers, types or argument types recorded |
| 06 Inheritance, dispatch & overloading | minimal | same-file dispatch methods bind as one set (medium); no type hierarchy, arity or dispatch narrowing |
| 07 Scoping & shadowing | partial | lexical narrowing of nested defs; no locals or shadowing; module boundaries ignored |
| 08 Macros, preprocessor & codegen | none | macros neither extracted nor expanded; macro head recorded as a call |
| 09 FFI & cross-language links | none | `ccall`/PyCall not linked |
| 10 Frameworks & entry points | partial | 5 rules: `julia-module-init` (node), `julia-base-dispatch` (root mark), `julia-test`/`flux`/`genie` detect-only |
| 11 Test harness entries | minimal | `julia-test` detect-only; generic `test_`/`Test` name roots only |
| 12 Dead-code analysis | minimal | degraded class, max medium; no exports or visibility; value-passed functions show as dead |
| 13 Builtins & stdlib tables | partial | 248-name `julia.txt`, reference-seeded; no type or cast tables |
| 14 Confidence & certainty | partial | high only from single-candidate intra-file or `import M`; some wrong highs (shadowing, module boundary) |
| 15 Dynamic use & reflection | none | no dynuse; `getfield`/`eval` are plain calls |
| 16 Known gaps & limitations | n/a | see list: `using` inert, qualified calls unbound, macros and functors missing, `include` not followed |
