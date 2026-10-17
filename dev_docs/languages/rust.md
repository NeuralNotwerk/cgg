# rust

The Rust plugin (`RustPlugin`, `crates/cgg-lang/src/plugins/rust.rs`, about 4,260 lines including inline tests) is cgg's most detailed language plugin. It uses the `tree-sitter-rust` crate from crates.io (`tree_sitter_rust::LANGUAGE`, rust.rs:51-53); nothing is vendored. One recursive `Walker` pass (rust.rs:464-716) builds a scope stack of crate root, modules, `impl` blocks and traits. In that pass it records definitions, call sites, `use` imports, struct fields (used for `self.field` typing), `let` types, attributes and macro data. Post-passes then add `self.<field>` local types (rust.rs:2363) and `ClassDecl` links for `Deref` targets and type aliases (rust.rs:2704). A small `macro_rules!` expander lives in `plugins/rust_macro.rs`. The plugin declares the signals `value_refs`, `attributes`, `exports`, `impls`, `test_defs`, `unreachable` and `visibility` (rust.rs:38-49). Resolution is not delegated to any generic resolver kind. Rust has the most dedicated logic in `cgg-resolve`: about 279 `"rust"` mentions in `cross_file.rs`, plus steps 3b, 3c, 4a, 4b and 4c, Rust-only visibility filtering, crate anchoring, trait-overload narrowing and the opt-in `--guess-unique-names`.

## 01. Detection & parsing

- **Extension `.rs`**: `fn main(){}` in `a.rs` is detected as `rust`. Evidence: rust.rs:35-37; detect.rs:679 test table `("a.rs", "rust")`.
- **Shebangs**: none. The plugin does not override `shebangs()`, so it inherits the default `&[]` (cgg-lang/src/lib.rs:298-300). A `#!/usr/bin/env rust-script` file without the `.rs` extension is not detected.
- **Content detection**: none (no header or content sniffing for Rust in detect.rs).
- **Grammar**: a single grammar, `tree-sitter-rust` (rust.rs:51). There is no per-file grammar switch (`ts_language_for` is not overridden).
- **Re-parse/fallback**: none. A file with parse errors is still extracted, with `parse_status = "error"` (cgg/src/lib.rs:457-461). There is no blanking pass like the C# `#if` one. Macro expansion is the only place the plugin parses again: it re-parses the *expanded text* of a `macro_rules!` invocation with one parser per file (`Walker::parser`, rust.rs:1671-1700).
- **Depth skip**: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep` (cgg/src/lib.rs:155, 451-455). This applies to every language.
- **Minified skip**: when any line exceeds `MINIFIED_LINE_BYTES` = 2000, `locals::collect` is skipped for the file (cgg/src/lib.rs:159, 478-486). This applies to every language.
- **Crate/module path derivation**: `rust_module_path` (rust.rs:161-248) walks up to the nearest `Cargo.toml` (`crate_dir_for`, rust.rs:250) and derives the crate name from the manifest. A `-` in the name becomes `_`.
  - `src/foo/bar.rs` becomes `foo::bar`.
  - `src/foo/mod.rs` becomes `foo`.
  - `tests/x.rs` becomes `tests::x`, and `tests/dir/main.rs` becomes `tests::dir`.
  - `benches/` and `examples/` get no segments.
  - Tests: `integration_test_file_qualified_under_crate_tests_module` (rust.rs:3840) and `integration_test_main_rs_under_subdir_qualified_by_dir_name` (rust.rs:3866).
- **File outside any Cargo crate**: the crate root becomes the literal `crate`. Fixture: `fn main(){ t.foo(); }` alone produced the node `crate::main`.
- **Cargo.toml is read but not parsed as a source file**: it is skipped as `unknown-extension` (fixture audit `file_skipped`). It is consulted only for the package name and for dependency renames (rust.rs:272-440).
- **Partial: the module path comes from the file location, not from `mod` declarations**. A `src/more.rs` that no `mod more;` declares is still analysed as module `more`. Fixture: `impl crate::Store { fn other_block }` in an undeclared `src/more.rs` produced the QN `alpha::more::crate::Store::other_block`, with the impl type text, `crate::` prefix included, embedded verbatim.

## 02. Callables extracted

Variant classification is in rust.rs:767-778, and QNs are joined by `qualified_name` (rust.rs:2675).

- **Free function** `fn foo() {}` becomes `FreeFunction`, QN `<crate>::<mod...>::foo`. Test `free_function` (rust.rs:3158).
- **`pub`/`async`/`unsafe`/`const` functions**. `async` (direct or inside `function_modifiers`) makes the variant `AsyncFunction` whatever the scope (rust.rs:747-768). `unsafe fn` and `const fn` are ordinary functions. Test `visibility_and_async_captured` (rust.rs:3986). Fixture: `Circle::konst` (const) and `Circle::uns` (unsafe) were both extracted as methods.
- **Inherent method** `impl Type { fn m(&self) {} }` becomes `InherentMethod`, QN `<crate>::Type::m`. Test `method_in_impl` (rust.rs:3172).
- **Trait-impl method** `impl Trait for Type { fn m() }` becomes `TraitMethod`, QN `<crate>::<Type as Trait>::m` (rust.rs:540-574). The trait name is kept in `DefRecord::base_types` (rust.rs:563-566). The driver derives `CallableNode::trait_impl_target` from the QN (cgg/src/lib.rs:2478). Test `trait_impl_method` (rust.rs:3605). Fixture: `fx_crate::<Circle as Shape>::area` with `trait_impl_target = Shape`.
- **Trait declaration, with or without a default body**: `trait T { fn decl(&self); fn dflt(&self) {} }`. The bodiless form is a `function_signature_item` and is recorded too (rust.rs:599-606). Both become `TraitDefaultMethod` with QN `<crate>::T::m`. Visibility is `Public` when the trait is plain `pub`, otherwise `Unknown` (rust.rs:789-801). Test `trait_items_inherit_visibility_rather_than_claiming_private` (rust.rs:3624).
- **Foreign declarations** `extern "C" { fn c_side(x: i32) -> i32; }` are recorded as a `function_signature_item` and become a private free-function node with no body. Fixture: `fx_crate::c_side` exists, and `calls_c -> c_side` is high.
- **Associated functions** `fn new() -> Self` are inherent methods (kind `method`). There is no separate `Constructor` variant for Rust. Fixture: `Circle::new` kind `method`.
- **Named closure** `let inc = |x| x + 1;` becomes `NamedClosure` (rust.rs:1303-1332; test `named_closure_is_callable`, rust.rs:3887). **Partial**: the QN uses the *module/impl* scope, not the enclosing function. Fixture: a closure `shadow_me` inside `main` got QN `f3::shadow_me`, the same QN as the free function `fn shadow_me`. Also `Holder::f` for a closure inside `Holder::run`.
- **Spawned closure / async block**: `tokio::spawn(async move { .. })`, `std::thread::spawn(|| ..)`, and any callee whose last segment is `spawn`, `spawn_blocking` or `spawn_local`.
  - These become `NamedClosure` nodes `closure_at_<line>` or `async_at_<line>`, nested under the enclosing function's QN and tagged `spawned`. A synthetic call edge runs from the spawner (rust.rs:1340-1446).
  - Tests: `tokio_spawn_extracts_async_block_as_disjoint_callable` (rust.rs:4001) and `thread_spawn_also_extracts_closure` (rust.rs:4066). Fixtures: `Holder::run::closure_at_40` and `f3::main::async_at_22`.
- **Other inline closures** (`.map(|x| ..)`, `set_hook(Box::new(|_| ..))`) are not nodes. Their calls are attributed to the enclosing function (test `inline_callback_closure_not_extracted_as_disjoint`, rust.rs:4048). Fixture: `hooked()` inside a `set_hook` closure became an edge from `Holder::run`.
- **Nested `fn` inside a function** is extracted. **Partial**: the scope stack does not track functions (rust.rs:1386-1388 comment), so `fn nested_inner` inside `Holder::run` got QN `Holder::nested_inner` and was classified as a *method* of `Holder`. Fixture: `fx_crate::Holder::nested_inner` kind `method`, resolved at medium via cross-file.
- **`macro_rules!` definitions** become a `FreeFunction` node with attribute `macro` and `Vis::Private` (rust.rs:1448-1475). Fixture: `fx_crate::make_getter` attributes `['macro']`.
- **Macro-generated items**: functions and impls produced by expanding an item-position invocation of a same-file `macro_rules!` are tagged `macro:<name>` (section 08). Fixture: `gen_getter` attributes `['macro:make_getter']`.
- **Overloads / clauses**: not applicable. Rust has no overloading. Several impls of one generic trait for one type (`<T as From<A>>::from`, `<T as From<B>>::from`) are distinct QNs and are narrowed at resolution time (section 06).
- **Operators**: `impl Add for Square { fn add }` is extracted as an ordinary trait method `<Square as std::ops::Add>::add` (fixture). Operator *uses* are not call sites (section 03).
- **Visibility mapping** `rust_vis` (rust.rs:3055-3067): `pub` becomes Public, `pub(crate|super|in ..)` becomes Internal, absent becomes Private. Impl-side trait methods are `Unknown`.
- **Not extracted**:
  - `const`/`static` items, structs, enums and enum variants. Tuple-struct constructors like `Outer(Inner)` are calls to a non-callable; with `--include-external` they become `<external>::Outer` (fixture f3).
  - Closures stored in fields (`W { cb: Box::new(|| ..) }`) have no node.
  - Proc-macro or derive-generated code is never seen.
  - Items produced by `macro_rules!` arms with repetition are not generated (section 08).

## 03. Call-site forms

`ref_from_call` (rust.rs:2019-2094) handles the forms below.

- **Bare call** `foo()` produces name `foo` with an empty receiver. `Ok(..)`, `Err(..)` and `Some(..)` are dropped (rust.rs:2035-2040). Test `call_expressions_captured` (rust.rs:3902).
- **Method call** `recv.m()` produces name `m` with `receiver_hint` = the receiver text. `receiver_text` (rust.rs:2263-2287) peels parentheses, `?` and `.await` up to 4 levels, maps a struct literal to its type path, and joins multi-line receivers.
  - Fixtures: `(it).deep()`, `self.inner.lock().unwrap().grow()` and `c.grow().grow()` all resolved.
  - Test `receiver_hints_are_normalized_and_guards_typed` (rust.rs:3336).
- **Path call** `a::b::c()`, `Type::assoc()`, `crate::x::f()`, `super::f()` and `self::f()` split into receiver `a::b` and name `c`, with `path_receiver = true` (rust.rs:2041, 2053-2061, 2086). Fixture: `crate::util::helper_fn()`, `util::helper_fn()` and `self::util::helper_fn()` were all high.
- **UFCS / qualified trait calls**: `<Circle as Shape>::area(&c)` resolved high to `<Circle as Shape>::area`. `Shape::area(&c)` resolved high to the declaration `Shape::area`. `Inner::deep(self)` was high. All are fixtures.
- **`Self::f()`** resolved high (fixture f3: `Self::new()`, `Self::assoc_self()`).
- **Turbofish** `f::<T>()`, `x.parse::<u32>()`, `a::b::<T>()`: the `generic_function` callee is unwrapped and recorded as the plain call (rust.rs:2021-2030). Test `turbofish_callee_is_recorded_like_the_plain_call` (rust.rs:4219). Fixture: `generic::<Inner>()` was high.
- **Chained calls** `a.b().c()`: each `call_expression` is visited (rust.rs:703-710), and the outer receiver is the inner call's text. Builder chains are deliberately not typed as the builder (test `builder_chain_is_not_typed_as_the_builder`, rust.rs:3538).
- **Macro invocations** `name!(..)` produce a `RefRecord` with `macro_call: true` (rust.rs:1854-1881).
  - `is_rust_noise_macro` drops `format`, `print*`, `write*`, `panic`, `assert*`, `debug_assert*`, `todo`, `unimplemented`, `unreachable`, `matches`, `vec`, the `log`/`tracing` macros, `anyhow`/`bail`/`ensure`, `json` and `dbg`, matched by last segment (rust.rs:2630-2673).
  - Fixture gap: the path macro call `alpha::amacro!()` from another crate produced no edge.
- **Calls inside macro token trees**: `println!("{}", f(x))` and `assert_eq!(p.id(), 1)`. `refs_from_token_tree` (rust.rs:1491-1660) treats an identifier followed by a parenthesised group as a call. It keeps the receiver for `self.x.m()`, `a.m()`, multi-segment paths and dot chains, and drops turbofish receivers.
  - Tests: `macro_token_tree_call_keeps_receiver` (rust.rs:4099), `macro_token_tree_call_keeps_multi_segment_receiver` (4133), `macro_token_tree_dot_call_keeps_full_path_receiver` (4170), `macro_token_tree_turbofish_receiver_is_dropped_not_emitted` (4198) and `macro_token_tree_dot_chain_keeps_self` (3469).
  - Fixture: `println!("{}", in_macro_arg(3))` was high.
- **Function values passed as arguments** (callbacks / registrars): `iter().map(mapper)`, `route("/", get(handler))`, `register(&f)`.
  - `refs_from_args` (rust.rs:2289-2360) records every bare `identifier`, `scoped_identifier` or `&identifier` argument of *any* call. It does not consult the per-language registrar verbs. The record carries the `VALUE_REF_HINT` sentinel receiver plus `context` (callee text) and `route` (`registrar::route_of`, registrar.rs:392).
  - These become `Via::Reference` edges, which are dropped unless `--reference-edges` or `--dead-code` is given. Fixture (`--dead-code`): `Holder::run -> mapper` medium `reference`.
  - Rust uses the sentinel receiver, not `RefRecord::value_ref` (value_refs.rs is written for the Java/C#/Python receiver-carrying form).
- **Synthetic spawn edge**: spawner to `closure_at_N`/`async_at_N` (section 02).
- **Literal argument type**: `RefRecord::arg_type` comes from a sole argument that is a literal (`&str`, `&[u8]`, `char`, `u8`, `bool`, suffixed numbers), a `format!`, or `.to_string()` (rust.rs:2097-2157). A variable or `self.field` argument (optionally `.clone()`d) becomes `ARG_VAR_PREFIX<name>`, settled later by type_hints (rust.rs:2160-2193). Test `literal_argument_types_are_recorded` (rust.rs:3743).
- **Arity**: not recorded. `ref_from_call` never sets `RefRecord::arity`, and Rust is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583, which lists `java`, `scala` and `cpp`).
- **Not recorded**:
  - Operator uses (`d + e`, `x[i]`, `*x`, `?`). Fixture: no edge to `<Square as Add>::add`, which is rooted instead (section 12).
  - Calls through a closure-typed field `(w.cb)()`. Fixture f3: no edge.
  - Paren-less calls (not applicable to Rust).
  - Calls written in a function body as `callee` of a non-identifier, non-path, non-field expression, such as `(get_fn())()` (rust.rs:2062: `_ => return None`).

## 04. Imports & module resolution

- **`use` forms** (`record_use` rust.rs:1883-1921, `expand_use_payload` rust.rs:2939-3023):
  - `use a::b::c;` and `use x::y as z;`. Test `use_imports_captured` (rust.rs:3921).
  - `use a::b::{X, Y as Z};`. Test `use_block_imports_flatten` (rust.rs:3938).
  - Nested groups `use a::{b::{X, Y}, c::Z};`. Test `nested_use_block_flatten` (rust.rs:3976).
  - `use a::b::{self, X};`. Test `use_self_in_block` (rust.rs:3957).
  - Globs `use a::*;` produce a `*` marker that the cross-file resolver treats as a wildcard. Fixture: `pub use inner::*` let `alpha::glob_fn()` resolve, but only at **medium**.
  - `pub(crate) use a::B;` records the path correctly (via the grammar's `argument` field). Test `restricted_visibility_use_records_the_path` (rust.rs:3807).
- **`pub use` re-exports** are tagged `pub-use` (test `pub_use_is_tagged`, rust.rs:3966). They become `FileFacts::exports` (rust.rs:126-144). Chains are followed in cross_file (`rewrite_rust_use_path` cross_file.rs:831; test `rust_pub_use_reexport_chain_through_crate_prefix`, cross_file.rs:7997). Fixture: `use alpha::exported_deep` (re-exported from `inner::deep`) resolved high.
- **`crate::`/`self::`/`super::` paths**:
  - In `use`, they are rewritten against the file's own module (`rust_own_module` cross_file.rs:801, `rewrite_rust_use_path` cross_file.rs:831).
  - In call receivers, they are handled by step 3c (cross_file.rs:6223) and the test `rust_super_receiver_rewritten_via_caller_module` (cross_file.rs:8463).
  - Stated limitation: in a file whose definitions all sit in one `impl`, `self::` rewrites to `module::Type` and fails closed (cross_file.rs ~785-800 doc comment).
- **Rust 2018 uniform paths to child modules**: in a module declaring `mod expand;`, `use expand::run` resolves to `<own>::expand::run` (`rust_child_module_path`, cross_file.rs:881).
- **`mod x;` declarations**:
  - Each records a `use self::x as x` import, so `x::f()` is never mistaken for an external crate. This includes `#[path = ".."] mod perf;` (rust.rs:508-529; test `lib_target_name_and_mod_declarations_are_in_scope`, rust.rs:3487).
  - Each also pushes a scope segment for inline `mod a { .. }` (test `mod_prefix`, rust.rs:3829).
- **Synthetic `crate-root` import** gives every file its crate name (rust.rs:68-74).
- **Cargo dependency renames** `common = { package = "other-common" }` are recorded as `use other_common as common` (rust.rs:76-88, 272-405; test `cargo_dependency_renames_are_parsed`, rust.rs:3507). Fixture: `common::thing()` was attributed to the external module `other_common` in the audit `unresolved_by_module`.
- **Step 3b intra-crate retry**: `mod::fn()` is retried with each prefix of the caller's QN (cross_file.rs:6162).
- **External-crate refusal (step 4c, Rust-specific; Rust is *not* in heads.rs's `WorkspaceVocab` list)**:
  - A path receiver whose head is no indexed crate, import or child module, or is `std`/`core`/`alloc`, is refused rather than fanned out (cross_file.rs:6741; `is_external_rust_head` 7124; `rust_external_import` 7173; `RustPathVocab`).
  - Tests: `rust_external_crate_head_produces_no_edge` (8562), `rust_one_segment_external_path_receiver_produces_no_edge` (8633), `rust_in_workspace_module_head_is_not_external` (8711), `rust_externally_imported_type_is_not_a_workspace_type` (9055) and `rust_reexport_only_crate_and_manifestless_files_are_not_external` (9257).
  - Fixture: `serde_json::from_str::<u8>("1")` produced no edge to the workspace `alpha::from_str`.
- **Crate anchoring**: a receiver typed with a crate path binds only in that crate (`narrow_rust_crate_head`, cross_file.rs:4348; test `a_rust_receiver_typed_by_crate_path_binds_in_that_crate`, 14398).
- **Visibility-aware reachability**: a step-1d or step-5 candidate must be visible under Rust's privacy rule. A private item is reachable only from its own file or a descendant module, and `::tests::` helpers only from the same tests module (`rust_candidate_reachable`, cross_file.rs:3714). Test `rust_private_candidates_filtered_by_visibility_and_tests_module` (8209).
- **`FileFacts::packages`**: not used for Rust. Crate identity comes from the `crate-root` import.
- **`not-in-imported-module` (modules.rs `ModuleIndex`)**: not applicable. Rust's module calls go through `by_qn` and steps 3, 3b and 3c rather than modules.rs (no Rust handling in modules.rs).
- **Known partial**: `#[macro_export]` is not modelled specially. Macro callables are exempt from the privacy filter instead (cross_file.rs:3707-3713; test `a_private_macro_callable_in_another_file_is_kept`, 10094).

## 05. Type inference & receiver typing

- **Parameters** `fn f(s: &Store)` give `s.get()`, which resolves through the typed owner. Parameters are parsed by `parse_colon_param_rust` (type_hints.rs:1793) and see through `Arc`/`Box`/`Mutex`/… chains (`SMART_POINTERS_AND_LOCKS`, rust_types.rs:67).
  - Tests: `rust_params_see_through_wrapper_chains` (type_hints.rs:4054) and `rust_parameters_split_only_between_parameters` (3812).
  - Fixture: `fn take(b: &dyn Backend) { b.put() }` resolved to `alpha::Backend::put` high, and `impl_take(t: &impl Shape)` to `Shape::area` high.
- **Locals are scoped, not file-wide** (Rust's own rule): the `let` in scope at the site, and a later binding shadows (`RustBindings::shadowed_since`). Tests `rust_locals_resolve_to_the_let_in_scope` (type_hints.rs:4113), `rust_receivers_follow_the_binding_in_scope` (2905) and `an_untyped_rust_rebinding_ends_the_typed_one` (2880).
- **`let x = Type::f(..)`** is settled against `f`'s declared return type (`settle_assoc_call_locals` type_hints.rs:1037, `rust_assoc_return` 963). Plugin side: `push_assoc_local` (rust.rs:1166) and test `path_call_locals_record_the_associated_function` (rust.rs:3673). Fixture: `let c = Circle::new(1.0); c.area()` resolved to `<Circle as Shape>::area` high.
- **Annotated and struct-literal lets**: `let d: Square = ..` and `let e = Square { .. }`. Test `annotated_and_struct_literal_lets_are_typed` (rust.rs:3707). Fixture: `d.area()` resolved to `<Square as Shape>::area` high.
- **Wrapper constructors**: `let x = Arc::new(Inner::new())` is typed through `DEREF_TRANSPARENT_WRAPPERS` (rust_types.rs:74). Collection constructors are not unwrapped. Tests `let_type_sees_through_deref_transparent_wrapper` (rust.rs:3197), `let_type_does_not_unwrap_collection_constructors` (3583) and `let_type_sees_through_wrappers_or_stays_untyped` (3271).
- **`.clone()`** inherits the known type (test `let_type_clone_inherits_known_type`, rust.rs:3230).
- **Return-type propagation** `let g = make_circle(); g.grow()` works through `build_return_type_map` and `RETURN_WRAPPERS` (rust_types.rs:92, `is_return_wrapper`). Fixture: `g.grow()` resolved to `Circle::grow` at **medium**.
- **Struct fields**: the post-pass emits `self.<field>` local types per method (rust.rs:1924-2016, 2363-2408).
  - Generic fields with exactly one trait bound use the bound (`struct App<B: Backend> { b: B }`). Associated types of a type parameter are skipped.
  - Test `async_methods_get_self_field_types_and_generics_use_their_bound` (rust.rs:3300).
  - Fixture: `self.b.put()` resolved to `alpha::Backend::put` high, and `self.s.get()` (`Arc<Store>`) to `Store::get` high.
- **Lock guards**: `self.inner.lock()` types as the contents (`GUARD_SUFFIX` "#guard" rust.rs:2864, `LOCK_WRAPPERS`, `rust_guard_base` type_hints.rs:1971; test `rust_lock_guards_type_as_their_contents`, 4085). Fixture: `self.inner.lock().unwrap().grow()` resolved to `Circle::grow` high.
- **`if let` / `while let` / `match` unwrapped bindings**: the plugin records them in `bind_unwrapped` (rust.rs:1204-1284) and `settle_unwrapped_bindings` (type_hints.rs:1847) settles them from the parameter, `let` or `Option`/`Result` source. Test `unwrapped_pattern_bindings_are_recorded` (rust.rs:4235).
  - **Partial in fixture**: `if let Some(h) = maybe_circle() { h.area() }` (a *function-call* source) did not type `h`. The site fanned out by name to `Shape::area`, `<Circle as Shape>::area` and `<Square as Shape>::area`, all at medium.
  - `if let Some(ref x) = opt` and `match opt { Some(y) => y.deep() }` reached `Inner::deep` only at medium.
- **`Deref` targets**: `impl Deref for Outer { type Target = Inner }` is recorded as a `ClassDecl` with `Outer` based on `Inner` (`rust_type_links`, rust.rs:2704-2782). `Target = T` (an impl parameter) becomes `#deref-arg`. Test `deref_targets_aliases_and_dot_calls` (rust.rs:3560). Fixture: `o.deep()` on `Outer` resolved to `Inner::deep` at medium.
- **Type aliases** `type A = B;` are recorded as a `ClassDecl` (rust.rs:2759-2772).
- **External single-argument wrappers** (`State<AppState>`, `web::Data<T>`) deref to the argument (step 4b, cross_file.rs:6703; `NOT_DEREF_TO_ARG` rust_types.rs:9). Test `rust_deref_wrapper_and_glob_and_renamed_heads` (cross_file.rs:8979).
- **Self calls across files** (step 4a, cross_file.rs:6668): `self.m()` reaches another `impl` block of the caller's type in its own crate. Test `rust_wasm_bindgen_methods_and_cross_file_self_calls` (9334). Fixture: `s.other_block()`, defined in another file's `impl`, was high.
- **Not handled**:
  - **Generic parameters with trait bounds in functions**: `fn generic_take<T: Shape>(t: &T) { t.area() }` was unresolved (`candidates-in-other-files`, 3 candidates, receiver `T`). The README lists this under "Potential future improvements: Generic trait-bound receiver typing (Rust)".
  - **Static calls on a type parameter** `T::make()` were unresolved (fixture f3, 2 candidates).
  - **Pattern-destructured parameters** `State(st): State<Arc<Store>>`: `st.get()` was unresolved (fixture ws line 9).
  - **`as` casts**: `receiver_text` does not peel `type_cast_expression` (rust.rs:2263-2287), and the shared `receiver::peel` is not used by Rust.
  - **Typed closure parameters**: `closure_types.rs` covers only Dart, C# and Scala, and `settle_closure_args` / `CALLBACK_TYPE_CONSTRUCTORS` list no Rust entry (stdlib.rs:143).
  - **`declared_type` nullability tables**: `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS` and `TOP_TYPES` have no Rust row. Rust uses its own `rust_types` wrapper tables instead.
  - Implicit receivers (Kotlin only) and weak receiver types (Python only): not applicable.

## 06. Inheritance, dispatch & overloading

- **Trait impls**: the QN carries `<Type as Trait>`, `base_types` = [Trait] (rust.rs:563-566), and `trait_impl_target` is derived by the driver (cgg/src/lib.rs:2478).
- **Typed calls landing on a trait declaration**: `dyn Trait`/`impl Trait` receivers bind to the declaration `Trait::m` at high (fixtures `dyn_take`, `impl_take`, `take`).
- **`--dynamic-dispatch`** (`dispatch::fanout`, dispatch.rs:38) adds declaration-to-implementation `Via::Dynamic` low edges, driven by `trait_impl_target`. It is forced on under `--dead-code` (cgg/src/lib.rs:243-250). Fixture: `Shape::area -> <Circle as Shape>::area` and `-> <Square as Shape>::area` low, with resolver `dispatch:fanout`.
- **`inheritance_fanout`**: Rust is **not** in `INHERITANCE_LANGS` (dispatch.rs:110-). There is no class inheritance, so this does not apply.
- **Trait default methods**: `self.helper_default()` inside a trait default method resolved high, intra-file (fixture). Step 4a also consults the type's traits' defaults (cross_file.rs:6668-6700).
- **Homogeneous trait set over the cap**: an over-cap Rust set made entirely of one workspace trait's impls binds to that trait's single declaration (`homogeneous_trait_declaration`, cross_file.rs:273; test `over_cap_homogeneous_trait_set_binds_to_the_declaration`, 8814).
- **Several impls of one generic trait for one type** (`<Bytes as From<&str>>`, `<Bytes as From<u8>>`) are narrowed by the literal `arg_type` (`narrow_trait_overloads` cross_file.rs:5041, `rust_impl_parts` 4982, `trait_arg_matches` 4998). Tests `rust_trait_overloads_respect_the_cap_and_the_argument_type` (9143) and `rust_try_from_pair_resolves_to_its_own_direction_only` (10523).
- **Local trait impls for std types are not indexed under the bare owner**: `impl From<X> for String` does not capture `String::from(..)` (`STD_CORE_TYPES` cross_file.rs:53, `is_std_type_local_trait_impl` 350). Tests `rust_std_from_call_does_not_bind_to_local_from_impl` (9747) and `std_type_local_trait_impl_not_captured_by_bare_owner_name` (9818).
- **Extension-trait impls on containers** (`impl MyExt for Vec<T>`) are reached by a value typed as the container (test `rust_value_typed_as_container_reaches_its_extension_trait_impl`, cross_file.rs:8754).
- **Enum-variant receivers** `MyEnum::Variant.m()` resolve to the owner's method (test `rust_enum_variant_receiver_resolves_to_owner_method`, 9583).
- **Duck-typed fan-out for untyped receivers** (step 5, cross_file.rs:6843) is bounded by `--fanout-cap`.
  - `RUST_UNTYPED_ADAPTER_METHODS` (`ok`, `err`, `as_deref`) are never fanned out (stdlib.rs:92; test `rust_result_adapters_are_not_fanned_out_by_name`, 14623). `stdlib/rust.txt` names are screened too.
  - Fixture: `unknown_thing.put()` produced medium edges to `Backend::put`, `<Mem as Backend>::put` and `<Disk as Backend>::put`.
- **`super`/`base`**: not applicable. Rust `super::` is a module path; evidence.rs:184 explicitly excludes Rust from `SuperBaseOutOfGraph` handling.
- **`member_of_calling_class`, `overrides_below`, `nearest_class_copies`**: not applicable. Rust bare calls never reach methods (`bare_call_is_never_a_method` includes `rust`, intra_file.rs:49-54).
- **Arity-based overload narrowing**: not applicable (no arity recorded; not in `OVERLOAD_ARITY_LANGS`).
- **Mixins / extension methods (C# style)**: not applicable.
- **Not handled**: a trait's method called through a generic bound in a function (`t: &T`), as covered in section 05. The README notes that trait-declaration in-degree and impls reached only through the trait are "the largest remaining false-positive class in `--dead-code` on Rust".

## 07. Scoping & shadowing

- **`FileFacts::locals`** comes from `RUST_SPEC` (locals.rs:286-315).
  - Binders: `parameter`, `closure_parameters`, `let_declaration`, `for_expression` (after its value), `let_condition` in `if`/`while` (after its value) and `match_arm`.
  - Functions: `function_item` and `closure_expression`. Destructuring patterns are covered.
  - Tests: `a_declaration_is_not_in_scope_in_its_own_initializer` (locals.rs:714-745, Rust cases `let now = now()`, `if let`, `while let`, `if a && let`, `for`) and the Rust case at locals.rs:786.
- **`shadowed-by-local`**: a bare call through a parameter or local is never another file's function. Intra-file, same-named definitions outside the binding's function are dropped (test `a_parameter_or_local_shadows_the_files_function`, intra_file.rs:1102). Macro invocations are never shadowed (CLAUDE.md, phase 3 description).
- **Lexical narrowing of nested definitions**: `rust` is in the lexical-narrowing allowlist (intra_file.rs:120-136).
- **Positional shadowing**: **not** applied to Rust; it covers only OCaml, F# and Lua.
  - Fixture f3: `shadow_me();` written *before* `let shadow_me = || 1;` was bound to the closure node rather than the free `fn shadow_me`.
  - Both nodes share the QN `f3::shadow_me` (see section 02), so the two edges both landed on the closure (start line 32).
- **Conditional definitions**: `#[cfg(feature = "a")] fn twin()` and `#[cfg(not(..))] fn twin()` are two nodes carrying the `#[cfg(..)]` attribute. A call binds to both at **medium** (fixture `calls_twin -> twin` x2, `cross-file:imports`). `DefRecord::conditional` is PHP-only, and `#[cfg]` twins are named as a reason a choice stays open (cross_file.rs:4487).
- **Rust variable shadowing for types**: `RustBindings::shadowed_since` (section 05).

## 08. Macros, preprocessor & codegen

- **`macro_rules!` expansion** (plugins/rust_macro.rs, 497 lines; `expand_item_macro` rust.rs:1671-1786):
  - Only same-file definitions are expanded, and only *item-position* invocations (`is_item_position`, rust.rs:2871: file or module level, or an impl/trait body).
  - Only arms without repetition `$( … )*` are used, with no nested expansion (`expanding` flag). A failed expansion leaves pre-expansion behaviour (rust_macro.rs:1-11).
  - Generated callables are tagged `macro:<name>`, with spans remapped into slices of the invocation and lines pointing at the invocation.
  - Test `same_file_item_macros_are_expanded_into_synthetic_callables` (rust.rs:3418). Fixture: `make_getter!(gen_getter);` produced the `gen_getter` node, and `uses_gen -> gen_getter` was high.
- **What `macro:<name>` means downstream**: the item is reachable by path and owner but is kept out of by-name fan-out and out of implicit-trait rooting (roots.rs:845-848), and it is capped at medium in dead code (`Evidence::MacroGenerated`, deadcode/mod.rs:893-897).
- **Not expanded**:
  - Repetition arms. Fixture: `rep!(rep_a, rep_b)` with `$($n:ident),*` generated nothing, and `uses_gen -> rep_a` produced no edge.
  - Macros from other files or crates (`#[macro_export]` ones included).
  - Proc macros, attribute macros and derives.
  - Expression-position macro bodies.
- **Macro definitions as callables**: `record_macro` (rust.rs:1448). A macro invoked by path outside any function becomes a `toplevel:macro-invocation` root (roots.rs:660-703; test near roots.rs:1268). A bare-name invocation at module level became a `toplevel:invocation` root (fixture: `make_getter`, `rep`).
- **`name_mentions`** (never edges, read only by dead code):
  - Serde attribute strings (`default`, `deserialize_with`, `serialize_with`, `skip_serializing_if`, `getter`, `with` producing `path::serialize`/`deserialize`) are recorded with `via = attr:<key>` (rust.rs:2195-2261).
  - Bare lowercase identifiers passed to an unexpanded item-position macro are recorded with `via = macro-arg` (`mentions_from_item_macro`, rust.rs:1809-1852).
  - Tests: `serde_attribute_strings_and_macro_arg_values_are_mentions` (rust.rs:3380) and `item_position_macro_arguments_are_mentions` (3784).
  - Fixture: `default_port` got evidence `name-collides-with-screened-site: attribute-string`.
- **Calls inside macro arguments** are extracted from token trees (section 03).
- **Macro-argument receiver retry** in cross_file: tests `macro_arg_ref_retries_bare_on_exactly_one_crate_wide_candidate` (10283), `macro_arg_ref_retry_declines_when_crate_wide_candidates_are_ambiguous` (10356) and `macro_arg_receiver_fanout_binds_only_a_single_survivor` (10745).
- **`#[cfg]` / conditional compilation**: not evaluated. Attributes are recorded verbatim and both twins stay (section 07).
- **Preprocessor / `#define`, templates, quote/unquote**: not applicable.
- **Derive-generated code**: invisible. Derived traits are handled only by rooting implicit-trait impls (section 12). The serde rule's gap text (rules.rs:7845) states that the generated `Serialize`/`Deserialize` impls are the only callers of hand-written `Visitor` methods.

## 09. FFI & cross-language links

- **Attribute capture**: `collect_attributes` (rust.rs:2905) reads the preceding `#[..]` siblings. `extern "C"` inside the function is added as the pseudo-attribute `extern:C` (rust.rs:811-818).
- **Classifier** `classify_ffi` (ffi.rs:643-677). Exports: `pyfunction`/`pymethods`/`pyclass` map to pyo3, `wasm_bindgen` to wasm-bindgen, `napi`/`module_exports` to napi, `no_mangle`/`unsafe(no_mangle)`/`export_name`/`extern:c` to c-abi, and `uniffi::export` to uniffi. `#[link]` maps to a c-abi *import*.
- **Pass B links an export to same-named callables in other languages** (ffi.rs:188-262). The link is callable to callable at **medium**, `Via::Ffi(family)`, with no site line, and the owners must match for methods. `napi` is restricted to JS/TS, `pyo3` to Python and `c-abi` to the C family. wasm-bindgen and uniffi are unrestricted (ffi.rs:694-707). Fixtures (ws3):
  - A C prototype `exported_c` was linked to `alpha::exported_c` (c-abi).
  - The `.pyi` stub `alpha.py_bare` was linked to `#[pyfunction] py_bare` (pyo3).
  - JS `napi_bare` was linked to `#[napi] napi_bare`.
  - JS `wasm_bare` *and the Python stub* `alpha.wasm_bare` were both linked to `#[wasm_bindgen] wasm_bare`, which shows that wasm-bindgen is not language-restricted.
- **Partial: path-qualified FFI attributes are not linked**. `#[pyo3::pyfunction]` and `#[wasm_bindgen::prelude::wasm_bindgen]` matched the `ffi-export` *framework rule* (entry nodes were minted), but `classify_ffi` keys on the whole path, so ffi.rs drew no Pass-B edge. Fixture ws2: no `ffi` edge for `py_exported`/`wasm_exported`, while the bare forms (ws3) were linked.
- **Partial: `#[pymethods] impl` members** get no FFI attribute themselves (the attribute is on the impl, and `collect_attributes` reads only the fn's own siblings). They become entry nodes only through `container_attributes_for("ffi-export","rust") = ["pymethods","uniffi::export"]` (rules.rs:11777) and are not Pass-B linked (fixture: `PyThing::meth` entry node, no ffi edge).
- **wasm-bindgen extern methods** `fn m(this: &T)`: the owner is read by `wasm_bindgen_method_owner` (cross_file.rs:165). Test `rust_wasm_bindgen_methods_and_cross_file_self_calls` (9334).
- **JNI: not supported for Rust**. `#[no_mangle] pub extern "system" fn Java_com_x_Cls_nat` is classified only as c-abi via `no_mangle`. `extern "system"` is not captured, because only the literal `extern "C"` is (rust.rs:814). No Java `native` to Rust link exists, and Java `native` is an *import* (ffi.rs:668). Fixture: `Java_com_x_Cls_nat` attributes `['#[no_mangle]']`, with only an ffi-export entry node.
- **P/Invoke, asm**: not applicable to Rust sources. The asm bridge in Pass A is C/C++ only (ffi.rs:36-175).
- **Descriptor links**: no Rust handling. The Protobuf link to host-language rpcs does not mention Rust: tonic services are a declared gap (rules.rs:2734, `tonic` gap text).
- **Markup code-behind**: not applicable (C# only).

## 10. Frameworks & entry points

All 30 rules with `language: "rust"` in `crates/cgg-core/src/frameworks/rules.rs` (no multi-language rule string includes Rust):

| rule id | line | kind | trigger | node |
| --- | --- | --- | --- | --- |
| axum | 1275 | network | import `axum`/`utoipa_axum`. Attrs `debug_handler`, `utoipa::path`. Registrars `route,get,post,put,delete,patch,head,options,any,nest` (handler inside `get(..)` inherits the `.route` path) | yes |
| actix-web | 1301 | network | import `actix_web`. Attrs `get,post,put,delete,patch,head,options,route,main,utoipa::path`. Registrars `route,service,to,default_service` | yes |
| rocket | 1329 | network | import `rocket`. Attrs `get..options,route,catch,utoipa::path` | yes |
| actix-actor | 1357 | queue | import `actix`. Base types `Actor,Handler,StreamHandler`. Methods `handle,started,stopped,handle_stream` | no (root mark) |
| ffi-export | 1444 | ffi | attrs `no_mangle, unsafe(no_mangle), export_name, wasm_bindgen, napi, pyfunction, pymethods, uniffi::export`. Containers `pymethods, uniffi::export` | yes |
| warp | 2683 | network | import `warp`. **Gap**: filter chains not enumerated | gap |
| tide | 2700 | network | import `tide`. **Gap** | gap |
| poem | 2717 | network | import `poem`/`poem_openapi`. **Gap** (`#[handler]` + `Route::new().at`) | gap |
| tonic | 2734 | network | import `tonic`. **Gap** (service traits are generated by tonic-build) | gap |
| tauri | 2751 | network | import `tauri`. Attr `command` | yes |
| clap | 2768 | cli | import `clap`. **Gap** (derive/builder dispatch by match or string) | gap |
| cloudflare-workers | 4637 | network | import `worker`. Attrs `event, durable_object` (event kind not carried into the name) | yes |
| rust-std-traits | 7717 | lifecycle | import `std/core/alloc`. Base types `Drop, Display, Debug, Default, From, TryFrom, Into, TryInto, Deref(Mut), Index(Mut), Iterator, IntoIterator, PartialEq, PartialOrd, Ord, Hash, arithmetic/bit ops, FromStr, AsRef, AsMut, Borrow, FromIterator, Extend, Error` with matching methods | no (root mark) |
| embedded-rt | 7813 | lifecycle | import `cortex_m_rt, riscv_rt, msp430_rt, avr_device, esp_riscv_rt, xtensa_lx_rt, embassy_executor`. Attrs `entry, interrupt, exception, pre_init, task, main` | yes |
| serde | 7845 | lifecycle | import `serde*`. **Gap** (attribute-string helpers become `name_mentions` only) | no |
| rust-runtime | 7862 | lifecycle | import `std/core/alloc`. **Gap** (closures to `set_hook`, `ctrlc`, `atexit`, `Box<dyn Fn>`). This rule's gap text says spawn closures have no node, but the plugin *does* mint nodes for `spawn`/`spawn_blocking`/`spawn_local` closures (rust.rs:1340), so the text is partly stale | no |
| salvo | 9036 | network | import `salvo*`. Attrs `handler, endpoint, craft`. Registrars `get..options, goal, handle` | yes |
| ntex | 9055 | network | import `ntex*`. Attrs `get..trace, route`. Registrars `route, service, to, default_service` | yes |
| gotham | 9075 | network | **Gap** (router-builder closure) | gap |
| iron | 9092 | network | **Gap** (`router!` macro) | gap |
| lambda-runtime | 9109 | network | import `lambda_runtime/lambda_http/aws_lambda_events`. Registrar `service_fn` | yes |
| shuttle | 9126 | lifecycle | import `shuttle_*`. Attrs `shuttle_runtime::main, shuttle_service::main` | yes |
| tokio-cron | 9151 | schedule | **Gap** (job body is a closure) | gap |
| cargo-test | 9173 | test | attrs `test, *_test, rstest, test_case, quickcheck` | yes (one harness node) |
| cargo-bench | 9191 | test | attr `bench` | yes |
| rtic | 9212 | lifecycle | attrs `init, idle, task` (no detect import) | yes |
| rmcp | 9231 | network | import `rmcp`. Attrs `tool, prompt`. Containers `tool_handler, prompt_handler` | yes |
| async-graphql | 9251 | network | import `async_graphql`. Containers `Object, ComplexObject, Subscription, MergedObject, Interface` | yes |
| juniper | 9269 | network | import `juniper`. Containers `graphql_object, graphql_subscription, graphql_interface` | yes |
| criterion | 9760 | test | import `criterion`. **Gap** (`criterion_group!` macro). The bench fn becomes only a `macro-arg` mention | gap |

- **`container_attributes_for`** (rules.rs:11771-11795): `ffi-export`, `async-graphql`, `juniper` and `rmcp`. The plugin feeds these through `FileFacts::attributed_containers` (rust.rs:548-560; test `an_attributed_impl_block_is_recorded_as_a_container`, rust.rs:3121; frameworks/mod.rs:2381 test).
- **`member_sources_for`, `signature_methods_for`, `markup_kinds_for`, `visibility_entries_for`, `self_module_markers_for`, `detect_calls`**: no Rust entries (rules.rs:11660-11870).
- **Registrar verbs**: the Rust plugin does not use the registrar-verb gate (`ExtractCtx` verbs). `refs_from_args` records every bare function-valued argument with `context` and `route`, and rules match on the callee. Fixture: `Router::new().route("/", get(handler))` minted `<framework-entry>::network::axum::get("/") -> beta::handler`.
- **Response-trait rooting**: `IntoResponse`/`Responder` impls are rooted only when some function returns that type outside a self-converting wrapper (`RESPONSE_TRAITS`, `SELF_CONVERTING_WRAPPERS` rules.rs:78-88; roots.rs:819-842, `rust:returned-response`).
- **Dead-code `FRAMEWORK_ATTRS`** (roots.rs ~185-208) root Rust functions carrying `tokio::main`, `actix_web::main`, `command`, `event_handler`, `panic_handler`, `alloc_error_handler` and the generic `route/get/post/…/task`.
- **Detection**: rules gate on the crate name in `use` paths (`detect`). There is no `detect_paths` or `detect_calls` for Rust.

## 11. Test harness entries

- **`cargo-test`** (rules.rs:9173): attributes `test`, `*_test` (suffix pattern), `rstest`, `test_case` and `quickcheck`, read by last path segment, so `#[tokio::test]`, `#[sqlx::test]` and `#[wasm_bindgen_test]` qualify. It mints ONE node, `<framework-entry>::test::cargo-test`, with an entry edge to each case. Fixture: edges to `tests::t_one` (`#[test]`), `tests::t_two` (`#[tokio::test]`) and `tests::it::it_works`.
- **`cargo-bench`** (rules.rs:9191): `#[bench]`.
- **Plugin-side `test_role`**: `rust_test_role` (rust.rs:3069-3100) marks last segment `test` or `*_test`, plus `bench`, `rstest`, `proptest`, `quickcheck` and `test_case`, as `TestRole::Case`. Test `test_attributes_are_read_by_their_last_segment` (rust.rs:3109). Dead-code `TEST_ATTRS` (roots.rs:211) plus the `_test` suffix (roots.rs:717) feed coverage.
- **Naming patterns**: none. `is_test_name("rust", _)` returns false: "a bare `test_` fn is not a test" (roots.rs:986).
- **Test-file gating**: only crate-level `tests/` or `benches/` containers count as test files (testfile.rs:69-74). Definitions there with no marker get `TestRole::Support` (cgg/src/lib.rs:636-643). Fixture: `benches/b.rs::bench_fn` has `test_role = support`.
- **Partial: helpers inside `#[cfg(test)] mod tests` are not test code**. The testfile.rs comment says the in-file `#[cfg(test)]` signal handles them, but no code path assigns a role from `#[cfg(test)]` on the enclosing module. Fixture: `fx_crate::tests::test_helper` has `test_role = None` and was reported as a **high** production dead-code finding.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: not applicable (Python, PHP and Ruby only; rules.rs:11806).
- **Not supported**:
  - `proptest! { .. }` and other macro-defined tests: the `proptest` attribute is listed, but the common macro form generates tests that are never expanded (unverified by fixture).
  - Criterion benches (gap rule; the bench functions are only mentions).
  - Doc tests.
- **QN collision**: `benches/*.rs` and `examples/*.rs` have no module segment (rust.rs:211-226), so their functions share the crate-root namespace with `src/lib.rs` (fixture: `fx_crate::bench_fn`).

## 12. Dead-code analysis

- **Signals extracted** (rust.rs:38-49; deadcode/caps.rs test at 312-317 asserts `visibility` and `attributes` are Full for Rust): visibility, attributes, value refs (`VALUE_REF_HINT`), exports (`pub use`), impls, test defs, and unreachable-after-terminator.
  - The unreachable check uses `cfg::RUST` (cfg.rs:57-70), whose terminators are `return_expression`, `break_expression` and `continue_expression` only. `panic!()`, `unreachable!()`, `todo!()` and `std::process::exit` are not terminators.
  - The fixture capability table reads `rust … analyzable vis yes attrs yes refs yes disp yes`.
- **Root kinds for Rust** (deadcode/roots.rs):
  - `builtin:main` for `main` and `_start` (`LANG_ENTRY` roots.rs:31).
  - `ffi:export` (classify_ffi exports plus `FFI_EXPORT_ATTRS` roots.rs:172-182).
  - `builtin:test-attr` (`TEST_ATTRS`).
  - `builtin:framework-attr` (`FRAMEWORK_ATTRS`).
  - `rust:returned-response` (roots.rs:819-842).
  - `rust:implicit-trait` for impls of `IMPLICIT_RUST_TRAITS` (roots.rs:34-80: `Drop`, `Display`, `Debug`, `Default`, `From`/`Into`/`TryFrom`/`TryInto`, `Deref(Mut)`, iterators, comparison traits, `Hash`, `Clone`, `Copy`, `Serialize`/`Deserialize`, `Error`, `FromStr`, operators, `Index(Mut)`, `Fn*`, `Future`, `Send`/`Sync`, `Write`, …). This excludes `macro:` impls (roots.rs:844-866; test `<F as Display>::fmt` at roots.rs:1141).
  - `framework:rule` (for example `rust-std-traits:base-type`; fixture: `<Circle as Display>::fmt` and `<Square as Add>::add` rooted this way).
  - `framework:entry-node` and `framework:test-harness`.
  - `toplevel:invocation` and `toplevel:macro-invocation`.
  - `lang:export` (any callable whose simple name is `pub use`d, roots.rs:890-915).
  - `rust:reexport` (`collect_rust_reexports`, roots.rs:917-938, 996-1018).
  - `builtin:lifecycle` from the cross-language `CONVENTIONAL_METHODS` (roots.rs:162-170). **Quirk**: it roots any Rust fn named `initialize`, `to_s`, `toString`, `equals`, `hashCode`, `Dispose` or `describe`. Fixture: the trait default `Shape::describe` was rooted as "`describe` is invoked by the runtime".
- **Visibility rules**: `Private`/`Internal` produce `private-visibility` evidence, which corroborates a finding. `Public` produces `public-visibility`, which caps the finding (deadcode/mod.rs:945-966). Fixture: `pub fn Holder::run` was medium while `private_never_called` was high.
  - A `pub` item in a library crate is **not** a root by itself, so it is reported at medium.
  - Trait members of a `pub trait` are Public, and impl-side trait methods are `Unknown` (no claim).
- **Confidence caps**:
  - `implements-external-contract`: a Rust impl of a trait not declared in-tree (deadcode/mod.rs:475-486).
  - `macro-generated` (mod.rs:893-897).
  - `name-collides-with-screened-site` for `attribute-string` (serde) and `value-position`. Fixture: `default_port` and `mapper` were medium.
  - `implemented-trait-surface` (fixture: `Shape::area`).
  - `name-matches-unresolved-site`.
  - The Rust visibility filter on doubt (unresolved sites must be able to reach the item).
- **`super-base-out-of-graph` doubt**: Rust is excluded, because `super::` is a module (evidence.rs:184).
- **BLIND_SPOTS** (deadcode/caps.rs:43-51) lists three items for Rust: "calls inside macro arguments (format!, writeln!, vec!, assert_eq!)", serde attribute strings, and "functions passed as values inside macro arguments". The first is now partly stale, because `refs_from_token_tree` extracts calls inside macro arguments (fixture: `in_macro_arg` was linked).
- **`reflection` cap**: not applicable (section 15).
- **Known false-positive classes**:
  - Trait-declaration methods with in-degree zero, and impls reached only via a trait (README "Potential future improvements").
  - `#[cfg(test)] mod` helpers reported as production code (section 11).
  - Closure bodies passed to non-spawn APIs (rust-runtime gap).

## 13. Builtins & stdlib tables

- **`stdlib/rust.txt`**: 346 lines (crates/cgg-core/src/stdlib/rust.txt; loaded at stdlib.rs:184, 225). It holds std type, trait and method names used to screen fan-out and to classify `--include-stdlib` exit nodes. Fixture f3: `<stdlib>::Box::new`, `<stdlib>::Vec::new`, `<stdlib>::String::from` and `<stdlib>::String::len`.
- **`rust_types.rs`** (cgg-core/src/rust_types.rs):
  - `NOT_DEREF_TO_ARG` (line 9)
  - `SMART_POINTERS_AND_LOCKS` (67)
  - `DEREF_TRANSPARENT_WRAPPERS` (74)
  - `FIELD_WRAPPERS` (82)
  - `LOCK_WRAPPERS` (86)
  - `RETURN_WRAPPERS` (92), plus `is_return_wrapper` for any `*Result`
  - `STRING_PRODUCING_MACROS` (98)
  - `STRING_PRODUCING_METHODS` (102)
- **`RUST_UNTYPED_ADAPTER_METHODS`** = `ok`, `err`, `as_deref` (stdlib.rs:92). These are kept out of `rust.txt` so that a workspace macro of that name still binds.
- **`STD_CORE_TYPES`** (cross_file.rs:53) guards local trait impls on std types.
- **`CAST_INTRINSICS` / `CONSTRUCTING_METHODS`**: no Rust entry (stdlib.rs:35-58).
- **Implicit packages** (`SCALA_IMPLICIT_PACKAGES`-like): none. `std`, `core` and `alloc` heads are always external (step 4c).
- **`--include-stdlib` / `--include-external`** add low-confidence `<stdlib>::…` and `<external>::…` exit nodes (fixture f3: `<external>::tokio::spawn`, `<external>::Vec<u8>::len`). Note that `Vec<u8>::len` on a `let v: Vec<u8>` was classified *external*, not stdlib.
- **Noise macros** (rust.rs:2630-2673) are dropped before resolution, so they never appear as unresolved or external.

## 14. Confidence & certainty

- **High** comes from `CertainCtx` and `OwnerFacts` in cross_file. For Rust:
  - Intra-file unique bindings.
  - Exact or import-anchored paths (`crate::util::helper_fn` and `use`-imported functions).
  - Typed receivers whose type comes from a declaration, annotation, struct literal or `Type::new` and names exactly one type anchored in the right crate (`rust_owner_certain` cross_file.rs:4713; `unique_owner_names` treats the crate as one site, 4397-4440).
  - Step 3b only when no other enclosing module of the caller holds the same relative path (cross_file.rs:6162-6200).
  - Fixtures: `Holder::run -> Circle::grow` (lock guard), `beta::main -> alpha::Store::open/get` (cross-crate), and `take -> alpha::Backend::put`.
- **Medium**:
  - Return-type-propagated receivers (`g.grow()`).
  - Pattern-bound receivers (`if let Some(ref x)`, `match`, `for`).
  - `Deref`-reached methods.
  - Glob-import resolution (`alpha::glob_fn()`).
  - Duck fan-out.
  - `#[cfg]` twins.
  - Nested fn via cross-file (`Holder::nested_inner`).
  - `super::fmt_helper()` from `util.rs`.
  - Value references and FFI Pass-B edges.
  - All are fixtures.
- **Low**:
  - `--guess-unique-names` (Rust only; `unique_name_guess` cross_file.rs:200-271, tagged `cross-file:name-unique`; test `unique_name_guess_is_opt_in_low_and_guarded`, 8894).
  - Dynamic-dispatch edges, framework-entry edges and exit nodes.
  - Fixture: `--guess-unique-names` did not bind `t.zz_only_here()` to the private `m::zz_only_here`; the guard refused it.
- A receiver typed only by a guess (`CertainCtx::weak_receiver`) is never high. Rust does not set `weak_receiver_type` (Python only).
- **Mermaid `?` marker**: an arrow with no high site gets a leading `?` in its label. This is generic, not Rust-specific.

## 15. Dynamic use & reflection

- **dynuse**: Rust has no reflective allowlist. Test `a_language_with_no_allowlist_yields_nothing` (dynuse.rs:228-232) asserts empty output for Rust, and Rust does not declare the `dyn_uses` signal (rust.rs:38-49).
- **`Any::downcast_ref`, `TypeId`, `libloading::Symbol` / `dlsym` by string**: not captured. Fixture: `let name = "send";` produced nothing.
- **Trait objects (`dyn Trait`)**: handled as declaration binding plus the opt-in `--dynamic-dispatch` fan-out (section 06). This is not reflection.
- **Module-load screen**: not applicable (`DYN_MODULE_LOADERS` has no Rust entries).

## 16. Known gaps & limitations

- **Generic bound receivers in functions**: `fn f<T: Tr>(t: &T) { t.m() }` and `T::assoc()` are unresolved (fixtures fx line 64, f3 line 10). This is a README "Potential future improvement".
- **Pattern-destructured parameters** (`State(st): State<..>`) are untyped (fixture ws line 9).
- **`if let Some(h) = func()`** does not type `h` from `func`'s `Option<T>` return, so it fans out by name (fixture fx line 54).
- **Nested `fn` QN**: a nested fn inside a method becomes `Type::nested` and is classified as a method (fixture `Holder::nested_inner`). The scope stack has no function segments (rust.rs:1386-1388).
- **Named closure QN** uses module scope, so it can collide with a same-named free fn. There is no positional shadowing (fixture f3 `shadow_me`).
- **`macro_rules!` with repetition, cross-file or cross-crate macros, proc macros and derives are not expanded** (rust_macro.rs:1-11; fixture `rep!`).
- **Path macro calls into another crate** (`alpha::amacro!()`) produced no edge (fixture ws).
- **Path-qualified FFI attributes** (`#[pyo3::pyfunction]`, `#[wasm_bindgen::prelude::wasm_bindgen]`) are not linked by ffi.rs (fixture ws2 vs ws3).
- **wasm-bindgen and uniffi Pass-B links are not language-restricted**: a Python stub got linked to a wasm export (fixture ws3; ffi.rs:694-707).
- **JNI** (`extern "system" fn Java_…`) is not modelled, and `extern "system"` is not recorded (rust.rs:814).
- **`#[cfg(test)] mod tests` helpers** have no test role, so they show up as high dead-code findings (fixture). testfile.rs:69-71 claims an in-file signal that the code does not provide.
- **`#[cfg]` is not evaluated**: twins both bind at medium.
- **Operators, `?`, indexing, `Deref` coercion and `Drop`** produce no call edges. Their impls are rooted instead (`IMPLICIT_RUST_TRAITS`).
- **Closures passed to non-spawn APIs and `Box<dyn Fn>` fields** have no node (`rust-runtime` gap, rules.rs:7862). That gap text is partly stale about `spawn`.
- **Framework gaps**: warp, tide, poem, tonic, clap, gotham, iron, tokio-cron, criterion and serde (rules.rs gap strings).
- **CFG unreachable** ignores diverging macros and calls (`panic!`, `unreachable!`, `process::exit`) (cfg.rs:57-70).
- **Crate module path comes from file location**: an undeclared `src/x.rs` is still module `x`, and an `impl crate::T` QN embeds `crate::T` literally (fixture `alpha::more::crate::Store::other_block`).
- **`self::`-relative `use` in a single-impl file** fails closed (stated limitation, cross_file.rs ~785-800).
- **`benches/` and `examples/` share the crate-root namespace** (rust.rs:211-226).
- **`CONVENTIONAL_METHODS`** roots Rust fns named `describe`, `initialize`, `equals` and others (roots.rs:162-170; fixture `Shape::describe`).
- **Stale `BLIND_SPOTS` entry** about calls inside macro arguments (caps.rs:43-51).
- **Unexplained receiver hint**: in fixture f3, the receiver of `t.zz_only_here()` (no `t` binding in scope) was reported with hint `T`. A minimal file did not reproduce it, so the cause is unverified.
- **Ignored tests**: none found for Rust (`grep -rn "#\[ignore" crates` showed no Rust-related hits).
- **CHANGELOG**: line 659 mentions "Scope and type follow-ups to WS14 (WS14b; Rust, Zig, Perl, PHP, …)". No open Rust-specific follow-up was identified beyond the README items above (unverified beyond grep).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | full | `.rs` only, one grammar, Cargo-aware module paths; no shebang; path comes from file location rather than `mod` declarations |
| 02. Callables extracted | partial | Fns, methods, trait decls and impls, spawned closures, macro-generated items. Nested fns and named closures get wrong or colliding QNs |
| 03. Call-site forms | full | Bare, path, method, UFCS, turbofish, macro token trees, fn-as-value args. No operators, closure-field calls or arity |
| 04. Imports & module resolution | full | `use` trees, `pub use` chains, globs, Cargo renames, `crate`/`self`/`super`, external-head refusal, privacy filter |
| 05. Type inference & receiver typing | partial | Strong for params, lets, fields, guards and Deref. Function generic bounds, destructured params and `if let` from calls are unresolved |
| 06. Inheritance, dispatch & overloading | full | Trait impls, `--dynamic-dispatch` fan-out, homogeneous-trait and arg-type narrowing; arity and class inheritance n/a |
| 07. Scoping & shadowing | partial | Scoped locals and lexical narrowing work. No positional shadowing for named closures; `#[cfg]` twins both bind |
| 08. Macros, preprocessor & codegen | partial | Same-file non-repetition `macro_rules!` only. No proc or derive macros, no `#[cfg]` evaluation |
| 09. FFI & cross-language links | partial | c-abi, pyo3, napi and wasm-bindgen with bare attributes. Path-qualified attributes are unlinked; no JNI; wasm and uniffi are unrestricted |
| 10. Frameworks & entry points | partial | 30 rules (19 enumerate, 11 declared gaps). Container attributes for pyo3, graphql and rmcp |
| 11. Test harness entries | partial | cargo-test and cargo-bench by attribute. `#[cfg(test)]` helpers are not test code; macro tests and criterion are gaps |
| 12. Dead-code analysis | full | Every signal is extracted, with rich Rust root rules. Known false-positive classes: trait-shaped code and cfg(test) helpers |
| 13. Builtins & stdlib tables | full | 346-line `rust.txt` plus the `rust_types` wrapper tables; no cast or construct entries (none needed) |
| 14. Confidence & certainty | full | Crate-anchored `rust_owner_certain`; opt-in low `--guess-unique-names` (Rust only) |
| 15. Dynamic use & reflection | none | No dynuse allowlist for Rust; trait objects are handled through dispatch instead |
| 16. Known gaps & limitations | n/a | 21 items listed above |
