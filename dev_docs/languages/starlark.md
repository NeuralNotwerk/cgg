# starlark

The Starlark plugin (`StarlarkPlugin`, `crates/cgg-lang/src/plugins/starlark.rs`, 223 lines) covers the Python-subset build language used by Bazel, Buck and Pants. It is one of the smallest plugins in the tree. A single recursive walker records `function_definition` nodes as free functions and `call` nodes as references, and turns `load("label", ...)` calls into `ImportRecord { kind: "load" }` (starlark.rs:53-68, 82-164). The grammar is the crates.io crate `tree-sitter-starlark` 1.3.0 (`Cargo.toml:117`, `Cargo.lock`, `crates/cgg-lang/Cargo.toml:56`), loaded as `tree_sitter_starlark::LANGUAGE` (starlark.rs:21-23). Nothing is vendored. The `LanguagePlugin` trait no longer has a `resolver_kind` method (grep of `crates/cgg-lang/src/lib.rs` finds none), so the plugin declares only `id`, `extensions`, `ts_language` and `extract`. Everything else uses the trait defaults: no shebangs (lib.rs:298-300), default `signals()` (lib.rs:304-306), no `max_line_bytes`, and a single grammar. Registered at `crates/cgg-lang/src/plugins.rs:158`. Resolution uses the generic intra-file and cross-file paths. The only Starlark-specific resolver code is the `"load"` arm in `cross_file.rs:1894-1916`.

Fixtures used below live in `<scratch>/parity/starlark/` (`t/`, `t2/`, `t3/`, `t4/`) and were run with `target/release/cgg` 0.11.0.

## 01. Detection & parsing

- **Extensions `.bzl`, `.star`, `.bazel`** (starlark.rs:18-20). Verified: fixture `t/` files `pkg/defs.bzl`, `script.star` and `pkg/BUILD.bazel` were analyzed with `detected_via: "extension:.bzl"` / `".star"` / `".bazel"` (audit `file_analyzed` events). `MODULE.bazel` and `WORKSPACE.bazel` match through `.bazel`.
- **Extensionless Bazel files: NOT supported.** A plain `BUILD`, `WORKSPACE` or `BUCK` file is skipped as `unknown-extension`. Verified: fixture `t/pkg/BUILD` gave `{"event":"file_skipped","path":"./pkg/BUILD","reason":{"kind":"unknown-extension"}}`. The bazel rule's own `gap` text says so too (rules.rs:6732). `detect.rs` has no filename-based rule for these files (grep for `BUILD|WORKSPACE|starlark` in `crates/cgg-lang/src/detect.rs` finds nothing).
- **Shebangs: none.** `shebangs()` keeps the trait default `&[]` (lib.rs:298-300). Verified: an extensionless file starting `#!/usr/bin/env starlark` (`t/noext`) was skipped as `unknown-extension`.
- **Content detection: none.** Starlark is not in `cgg-lang::detect`'s content-detected set. The `detect_prefixes.rs` integration test builds a Starlark import probe `load("{prefix}", "sym")` in a `.bzl` file (`crates/cgg/tests/detect_prefixes.rs:89`), but that tests framework-detection prefixes, not language detection.
- **Grammar:** a single grammar with no per-file dialect (`ts_language_for` default, lib.rs:322-324).
- **Re-parse / fallback: none.** A file with syntax errors is still extracted from the tree-sitter error-recovered tree, with `parse_status: "error"`. Verified: in `t4/p/user.bzl`, an unterminated `x = [1, 2` gave `parse_status: "error"` and the three calls before it still resolved.
- **Skip conditions:** only the generic ones. `MAX_TREE_DEPTH` = 4,000 (`crates/cgg/src/lib.rs:155`, checked at lib.rs:451), plus the walker's binary/size classification. No `max_line_bytes` override. The minified-file rule (average line over 1,000 bytes) applies only to confidence (`cross_file.rs:4545-4550`), not to skipping. Nothing here is specific to Starlark.

## 02. Callables extracted

- **Top-level `def`:** `def my_macro(name, **kwargs): ...` becomes a `DefVariant::FreeFunction` with `simple_name == qualified_name == name` (starlark.rs:82-108). `signature_hint` comes from `super::extract_signature` (starlark.rs:103) and reads like `"def _impl(ctx)"` (JSON output, fixture `t/`). Test: `tests::def_and_call` (starlark.rs:188-196).
- **Nested `def`:** `def outer(): def nested(): ...` is extracted because the walker recurses into function bodies (starlark.rs:55-58). The qualified name is the bare `nested`, with **no owner or enclosing-function prefix**. Verified: fixture `t/pkg/defs.bzl` has a callable `nested` and an edge `my_macro -> nested` (intra-file, high).
- **Conditional `def`** (inside `if`) is extracted like any other. Verified: `only_x2` in `t2/c/z.bzl`. It is not marked `DefRecord::conditional`.
- **Qualified names never carry a file or module path.** Two `.bzl` files that both define `dup` produce two callables with the same `(starlark, "dup")` qualified name (fixture `t2/`). Consequences are covered in sections 04 and 14.
- **Lambdas: not callables.** `f = lambda y: java_helper(y)` creates no node. Calls inside the lambda are attributed to the enclosing `def`. Verified: `my_macro -> java_helper` is rendered `3x`, and the three sites are lines 14 (direct), 16 (inside the lambda) and 20 (inside a list comprehension). A lambda at module level (`struct(foo = lambda: None)` inside a def, or a top-level lambda) likewise has no node.
- **`rule(...)`, `macro(...)`, `provider(...)`, `aspect(...)`, `repository_rule(...)`, `module_extension(...)` results bound to a name** (`my_rule = rule(implementation = _impl)`): **not callables.** Only `def` is recorded. `my_rule` is invisible as a target, and the `rule(...)` call itself is a top-level site with no source (section 03).
- **Methods, constructors, properties, operators, overloads, abstract members:** not applicable. Starlark has no classes. `struct(...)` members are values, not definitions.
- **Visibility:** `visibility` is always `""` (starlark.rs:104). The `_private` naming convention is not recognised as private.
- **Attributes:** always empty (starlark.rs:105). Starlark has no decorators.

## 03. Call-site forms

- **Bare call:** `java_helper(name = name)` gives `RefRecord { name, receiver_hint: "" }` (starlark.rs:112-113, 157-163). Test: `tests::def_and_call`.
- **Attribute call:** `native.cc_library(name = 'x')` gives `name = "cc_library"`, `receiver_hint = "native"` (starlark.rs:114-126). Test: `tests::attribute_call` (starlark.rs:211-222). The receiver is the raw source text of the object (`ctx.actions`, `"a,b"`, `helper_inner().foo()`). Verified in fixture `t/` audit `external_calls`.
- **Chained call:** `helper_inner().foo().bar()` records `helper_inner` (resolved, high), `foo` with receiver `helper_inner()`, and `bar` with receiver `helper_inner().foo()`. The last two are external (fixture `t/`, line 26). The receiver text is never typed.
- **Call whose callee is itself a call or subscript: NOT recorded.** `getattr(native, "genrule")(name = "g")` and `handlers["x"](...)` hit the `_ => return` arm (starlark.rs:127). Only the inner `getattr` call is recorded. Verified in fixture `t/` line 25: only `getattr` appears.
- **`load(...)`** is consumed as an import and is **not** recorded as a call reference (starlark.rs:133-155).
- **Top-level calls** (every BUILD-file macro invocation, `my_rule = rule(...)`, `my_macro(name = "top")`) are recorded, but they have no enclosing callable. They become unresolved sites with `src: null` and stage `no-enclosing-callable`, or `candidates-in-other-files` / `no-candidate-in-file` when the name binds elsewhere. Verified: fixture `t/` `unresolved`: `defs.bzl:31 my_macro no-enclosing-callable`, `script.star:4 main no-enclosing-callable`, `BUILD.bazel:2 my_macro candidates-in-other-files`. There is no synthetic module or toplevel callable, so **BUILD files contribute no edges at all.**
- **Constructor calls:** not applicable (no classes). `struct(...)`, `provider(...)` and `DefaultInfo(...)` are ordinary bare calls and are classified external (fixture `t/`).
- **Explicit generics, call operator, paren-less calls:** not applicable to the language. `RefRecord::paren_less` is never set.
- **Value references / callbacks passed as values: NOT recorded.** `rule(implementation = _impl)`, `struct(fn = my_macro)` and `f = rel_target` create no `value_ref` records. `ExtractCtx` is ignored entirely (`_ctx`, starlark.rs:27). Verified: with `--reference-edges`, fixture `t/` gains no `_impl` edge, and `_impl` stays a dead-code finding.
- **Calls through a local alias:** `f = rel_target; f()` records a call to `f`, which goes external (`t4/p/user.bzl:9`, `X 9 f no-candidate-in-file`).
- **Registrar arguments:** none. The plugin does not call `registrar.rs`.
- **Arity:** `RefRecord::arity` is never set. Keyword arguments are not recorded, so `signature_hint` keyword narrowing cannot apply.
- **Calls in default parameter values, comprehensions and lambdas** are recorded because the walker visits every `call` node in the tree (starlark.rs:53-68). Verified for the comprehension and lambda cases (fixture `t/`, lines 16 and 20).

## 04. Imports & module resolution

- **`load("//pkg:file.bzl", "sym", ...)`** gives `ImportRecord { kind: "load", path: <label>, alias: "" }` (starlark.rs:133-155). Test: `tests::load_captured_as_import` (starlark.rs:198-209). Only the first string argument (the label) is kept. **The symbol list and aliases (`h2 = "other_helper"`) are discarded.**
- **Resolution is include-like** (`cross_file.rs:1894-1916`). The label has its leading `//` trimmed and `:` replaced by `/`. Two paths are tried, `cleaned` and `cleaned.bzl`, through `collect_include_defs` (depth 4). Every definition in the matched file goes into `direct_imports`, whatever symbols the `load` names (comment at cross_file.rs:1896-1899). Matching first tries the includer's directory joined with the path, then any file whose path ends with that suffix, lowest FileId first (cross_file.rs:3456-3483).
  - Workspace-absolute label `//common:macros.bzl`: works. Verified, fixture `t/`: `my_macro -> java_helper` is high, resolver `cross-file:imports`.
  - Extensionless label `//q:lib`: works through the `.bzl` retry. Verified, `t4`: `u -> other_only` is high.
  - **Aliased load `load(..., h2 = "other_helper")`: NOT supported.** `h2()` is classified external, and `other_helper` is then reported dead. Verified, fixture `t/`, line 15.
  - **Symbols not named in the load are still imported**, because every def of the loaded file becomes visible. Verified, `t4`: `ext_target` is reachable through `load(":lib.bzl", "rel_target")`.
  - **Package-relative label `:lib.bzl`:** resolved, but only at medium in `t4` (`u -> rel_target medium`). The cleaned path is `/lib.bzl`, which joins as an absolute path. Why it still matches, and why it lands at medium, is unverified.
  - **External-repo label `@other//p:lib.bzl`:** no external-repo awareness. The leading `//` trim does not apply to `@repo//...` labels, so in `t4` that particular load matched nothing on its own, and `ext_target` bound to the in-tree `p/lib.bzl` through the other load. An `@repo` label is never refused as `external-module`.
  - **Wrong-file binding.** The plugin records unqualified qualified names, and the `direct_imports` lookup goes through `lookup_with_reexports(lang, qn, by_qn, …)` (cross_file.rs:5710-5755), which keeps **one** callable per qualified name. When two files define the same name, the call can bind to the file that was **not** loaded. Verified, fixture `t2/`: `c/z.bzl` loads `//a:x.bzl`, yet `caller -> dup` bound to `./b/y.bzl` at **high**. Renaming `b` to `zz` (`t3/`) gave the same wrong binding, to `./zz/y.bzl`. The output is deterministic across 3 runs (identical md5 of the mermaid output).
- **Calls with no load:** a bare call to a name defined in one other `.bzl` file that the caller never loaded stays unresolved (`candidates-in-other-files`, `candidates: 2`, `t2/c/w.bzl`). With one candidate the reason is the same (`BUILD.bazel:2 my_macro`, `candidates: 1`, fixture `t/`).
- **`FileFacts::packages`:** never set. Starlark does not join the `heads.rs` external-module refusal (`WorkspaceVocab`/`FileHeads` cover Python/JS/TS/Go/Java/Kotlin/C#/C++ only; grep for `starlark` in `heads.rs` and `modules.rs` finds nothing). There is no `not-in-imported-module` refusal and no `ModuleIndex` entry.
- **Manifest reading:** none. `MODULE.bazel` `bazel_dep`, `WORKSPACE` `http_archive` / `local_repository` and `.bazelrc` are not read.
- **Receivers that name a loaded struct** (`paths.join` after loading `paths` from skylib) are classified `external` (`t/` line 7), even when the struct's file is in the tree. Struct members are never callables.

## 05. Type inference & receiver typing

- Not supported. The plugin records no parameter types, local types, field types, return types, `scoped_types`, `typed_receiver`, `implicit_receiver` or `weak_receiver_type` (starlark.rs:157-163 sets only `name`, `receiver_hint`, `site_line` and `site_byte`). Starlark has no type annotations in the Bazel dialect, and the grammar's optional type syntax is not read.
- `type_hints` has no Starlark-specific code (grep for `starlark` in `crates/cgg-resolve/src/type_hints.rs` finds nothing). File-wide generic propagation could only act on records the plugin never produces.
- `ctx.actions.write(...)`, `ctx.attr.x(...)` and similar calls are external through the name screen, never typed (fixture `t/`).

## 06. Inheritance, dispatch & overloading

- Not applicable. Starlark has no classes, bases, `super`, interfaces, traits or overloads.
- Not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS`, and has no `trait_impl_target` (grep for `starlark` in `dispatch.rs` and `cross_file.rs` finds only the `load` arm). `--dynamic-dispatch` adds nothing for Starlark (fixture `t/` run with `--dynamic-dispatch`).
- The closest Starlark analogue of dispatch, `provider`/`struct` field calls (`s.fn("z")`), goes external. Verified, fixture `t/` line 23: `<external>::s::fn` under `--include-external`.

## 07. Scoping & shadowing

- **`FileFacts::locals`: not recorded.** `locals.rs` has no Starlark node kinds, and the plugin never calls it (grep finds no `starlark` in `crates/cgg-lang/src/locals.rs`).
- **No `shadowed-by-local`.** A parameter `def caller2(dup): dup()` still binds the parameter's call to a cross-file `dup` at high. Verified, `t2/`: `caller2 -> dup (./b/y.bzl) high cross-file:imports`.
- **No lexical narrowing of nested definitions.** Starlark is not in the narrowing set (JS/TS, Lua, OCaml, F#, Scala, Kotlin, Swift, Rust, Dart, Haskell, Julia, per CLAUDE.md). A nested def is visible file-wide by its bare name. Verified, `t2/c/z.bzl`: `caller` calls `only_x()` and binds at high to the `only_x` nested inside a different function, `shadow()`, rather than to the loaded `//a:x.bzl` `only_x` (`caller -> ('only_x','./c/z.bzl') 4 high intra-file`).
- **No positional shadowing**, and no `DefRecord::conditional` for `if`-guarded defs (starlark.rs:95-107 sets neither).

## 08. Macros, preprocessor & codegen

- Not applicable in the C or Rust sense. Starlark has no preprocessor. "Macros" in Bazel are ordinary `def` functions and are extracted as free functions (section 02).
- Bazel **symbolic macros** (`macro(implementation = _impl, ...)`) and legacy macros invoked from BUILD files are not modelled. The invocation sites are top level, or in extensionless BUILD files that are not analyzed (sections 01 and 03).
- No `macro:<name>` tagging and no `name_mentions` (starlark.rs never touches `facts.name_mentions`).

## 09. FFI & cross-language links

- None. `ffi::link_ffi` has no Starlark path (grep finds no `starlark` in `crates/cgg-resolve/src/ffi.rs`). `descriptor::link_descriptors` has no Starlark target.
- A `cc_library`/`py_library`/`java_library` target's `srcs` are not linked to the host-language files (string list arguments are ignored).
- Markup code-behind is not applicable.

## 10. Frameworks & entry points

- **`bazel`** — `language: "starlark"`, `kind: TrustKind::Lifecycle`, `node: true` (rules.rs:6696-6736). The trigger is **detection only**: `detect` holds 22 import prefixes (`@bazel_skylib`, `@bazel_tools`, `@bazel_gazelle`, `@rules_python`, `@rules_cc`, `@rules_java`, `@rules_go`, `@io_bazel_rules_go`, `@rules_rust`, `@rules_shell`, `@rules_proto`, `@rules_pkg`, `@rules_license`, `@rules_testing`, `@rules_foreign_cc`, `@rules_jvm_external`, `@rules_nodejs`, `@rules_oci`, `@aspect_bazel_lib`, `@aspect_rules_js`, `@com_google_protobuf`, `@platforms`). `detect_paths`, `detect_calls`, `attributes`, `registrars`, `base_types` and `methods` are all `NONE`, and `string_targets` is false. **It enumerates no entries**, and its `gap` text is printed under "seen, no rules". Verified, fixture `t/`: `seen, no rules bazel — found in 1 file(s), entries NOT enumerated`, triggered by `load("@bazel_skylib//lib:paths.bzl", "paths")`.
- `benchmark.sh` lists `~bazel` (an expected-unverified app marker) on `app-kythe-bazel` (scripts/benchmark.sh:325). The Starlark benchmark repo is `starlark-skylib` (benchmark.sh:94, 399; update-readme-stats.sh:95; README.md:1346 shows `bazel-skylib | starlark | 93 | 44 | 0% | 10ms`).
- No other rule names `starlark` (grep for `starlark` in rules.rs matches only line 6698).
- `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`: none apply (Starlark has no attributes, classes or markup).
- Registrar verbs: none. `rule(implementation=...)`, `aspect(...)`, `repository_rule(...)`, `module_extension(...)`, `transition(...)`, `provider(init=...)` and `tag_class` are not recognised. This is the gap the rule itself documents.

## 11. Test harness entries

- None. No rule of kind `test` names Starlark. `analysistest.make(_impl)`, `unittest.make(_test_impl)` (bazel-skylib `unittest.bzl`) and `*_test` macros are not recognised, and `TEST_CLASS_MEMBERS_ARE_ROOTS` does not apply (no classes).
- No test-file gating. `_test.bzl` / `tests/` files are analyzed like any other file.

## 12. Dead-code analysis

- **Language class `script-driven`.** Starlark is listed in `SCRIPT_DRIVEN` (`crates/cgg-resolve/src/deadcode/caps.rs:27-36`, also bash, cmake, nix, hcl, powershell, r, perl), which sets `LanguageClass::ScriptDriven` (caps.rs:166-167).
- **Every finding is capped at `low`.** `Evidence::LanguageIsScriptDriven` maps to `Confidence::Low` (`crates/cgg-core/src/deadcode.rs:314-316`). Root-dependent categories are suppressed (`SuppressionReason::ScriptDriven`, `crates/cgg-resolve/src/deadcode/mod.rs:811-812`), so at the default threshold nothing is shown. Verified, fixture `t/`: `findings 0 shown (high) · 3 withheld (low)`. The capability row is `visibility none, attributes none, value_references none, dispatch none, exports none, test_tagging none, max_confidence medium, root_rules_active ["toplevel:invocation"]`. Each finding carries the evidence `language-is-script-driven`, `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges` and `no-unresolved-site-with-name`.
- **The one active root kind is `toplevel:invocation`** (`deadcode/roots.rs:575-612`). A callable is rooted when an unresolved site with reason `NoEnclosingCallable` / `ValueRefNoEnclosing` carries its `(language, simple_name)`. Verified: `main` (from `script.star:4`) and `my_macro` (from `defs.bzl:31`) were rooted.
  - **A BUILD-file invocation of a loaded macro does not root it.** That site's reason is `candidates-in-other-files`, not `no-enclosing-callable`. Verified: after deleting `defs.bzl:31`, the `BUILD.bazel:2 my_macro(...)` call still left `my_macro` among the findings `['_impl','other_helper','unused_one','my_macro']`.
- **False findings by construction:** `_impl` functions named only as `implementation =` arguments (no value refs), and symbols called through a `load` alias (`other_helper` via `h2`). Both verified in fixture `t/`.
- No visibility rules: `_name` (load-private in Bazel) is not treated as file-private, and public names are not export roots.
- Unresolved-doubt handling is the generic one (`NameMatchesUnresolvedSite`). Starlark names reach the external bucket readily, so few unresolved sites exist to supply doubt.
- `--dead-code` forces `reference_sites`, but the plugin ignores `ExtractCtx`, so it has no effect here.

## 13. Builtins & stdlib tables

- **No `stdlib/starlark.txt`.** It is absent from `crates/cgg-core/src/stdlib/` and from the `stdlib_names` table (stdlib.rs:184-213 and the map at ~221-255 cover 30 languages; Starlark is not one). There is no fallback to `python.txt`.
- As a result, `print`, `str`, `len`, `struct`, `depset`, `getattr`, `fail`, `select`, `DefaultInfo` and `native.*` are classified **external**, never **stdlib**. Verified, fixture `t/`: `0 stdlib, 17 external`. With `--include-stdlib --include-external` they appear as `<external>::print`, `<external>::struct`, `<external>::depset` and so on. `--include-stdlib` alone adds nothing.
- No `rust_types`-like tables, no `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` / `MAYBE_TYPE_WRAPPERS` entries, no implicit packages.

## 14. Confidence & certainty

- **`high` is reachable** through intra-file direct binding and through `cross-file:imports` (the `direct_imports` path). Verified: fixture `t/` `java_helper -> _private high intra-file` and `my_macro -> java_helper high cross-file:imports`.
- **High is issued even when wrong.** Fixture `t2/`: the wrong-file `caller -> dup` and the shadowed parameter call `caller2 -> dup` are both high, and the nested-def binding `caller -> only_x` (a def nested inside another function) is high (intra-file). `CertainCtx::path` should refuse a qualified-name collision (`cross_file.rs:4485-4497`), and `(starlark, "dup")` is defined twice. Why the collision check did not lower these to medium is unverified.
- **Medium** observed: the package-relative label load (`t4`, `u -> rel_target medium`). The cause is unverified.
- No `low` edges observed (no `--guess-unique-names` path; that flag is Rust-only).
- The mermaid `?` marker applies generically to arrows with no high site. None appeared in fixture `t/`.

## 15. Dynamic use & reflection

- None. `dynuse` covers Python, JS, Java, C#, Ruby and PHP only (CLAUDE.md), and grep finds no `starlark` in `crates/cgg-lang/src/plugins/dynuse.rs`.
- `getattr(obj, "name")` is recorded only as a call to `getattr`, which goes external. The call it returns is dropped (section 03). It does not cap same-named dead-code findings.
- `native.existing_rule(...)` / `native.existing_rules()` and string-keyed `select({...})` are not modelled.

## 16. Known gaps & limitations

- The `bazel` rule's `gap` text (rules.rs:6732) is the explicit record. It names three gaps: no entry rules for `implementation =`, `init =`, `resolve_fn`/`toolchain` hooks or BUILD-file macros; implementation functions named in argument position are not recorded as references; and plain `BUILD` files are not analyzed.
- Comment at `cross_file.rs:1896-1899`: "tracking the actual symbols list would require a separate Starlark-aware import representation". The load symbol list and aliases are ignored.
- README language-table row (README.md:519): `Starlark | load("//path:f.bzl", …) | — | def/call/attribute; Bazel/Buck/Pants`.
- No Starlark entries in CHANGELOG.md (grep for `starlark|bazel` finds none).
- Gaps found while cataloguing, each verified by fixture:
  1. **Wrong-file `high` bindings** when two `.bzl` files define the same name. The qualified names carry no module, and the `direct_imports` → `by_qn` lookup keeps one copy (`t2`, `t3`).
  2. **Aliased load symbols are lost**, which gives false external calls and false dead code (`t`, `h2`).
  3. **Top-level calls produce no edges**, and a BUILD-file invocation of a loaded macro does not root it (`t`).
  4. **Extensionless `BUILD` / `WORKSPACE` / `BUCK` files are skipped** (`t/pkg/BUILD`).
  5. **No locals**, so parameters do not shadow cross-file names, and nested defs leak file-wide (`t2`).
  6. **No value references** (`implementation = _impl`, `struct(fn = f)`, `f = g; f()`) (`t`, `t4`).
  7. **Calls whose callee is a call or subscript are dropped** (starlark.rs:127).
  8. **No stdlib table**, so every builtin counts as external (`t`).
  9. **Lambdas are not callables** (`t`).
  10. **`@repo//` labels are not treated as external repositories** (`t4`).
- Starlark tests: three inline unit tests (`def_and_call`, `load_captured_as_import`, `attribute_call`) plus the `detect_prefixes.rs` probe. No integration fixture, and no ignored tests (grep for `#[ignore]` in starlark.rs finds none).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.bzl`/`.star`/`.bazel` only; extensionless BUILD/WORKSPACE/BUCK and shebangs skipped |
| 02. Callables extracted | partial | `def` (incl. nested, bare qn) only; no lambdas, no `rule()`/`macro()` targets, qns carry no module |
| 03. Call-site forms | partial | bare + attribute + chained; no value refs, no arity, call-of-call dropped, top-level sites unattributed |
| 04. Imports & module resolution | partial | `load` is include-like by label; symbol list/aliases ignored; wrong-file binding on duplicate names; no `@repo` externality |
| 05. Type inference & receiver typing | none | no types, locals or receiver typing recorded |
| 06. Inheritance, dispatch & overloading | n/a | language has no classes/overloads; not in any dispatch list |
| 07. Scoping & shadowing | none | no locals, no shadowing, nested defs visible file-wide |
| 08. Macros, preprocessor & codegen | n/a | no preprocessor; Bazel macros are plain defs; no name_mentions |
| 09. FFI & cross-language links | none | no link to host-language srcs or any FFI path |
| 10. Frameworks & entry points | minimal | `bazel` rule detects only (22 prefixes), enumerates no entries |
| 11. Test harness entries | none | no `analysistest`/`unittest` rule |
| 12. Dead-code analysis | minimal | script-driven, capped low and withheld; only `toplevel:invocation` roots (same-file only) |
| 13. Builtins & stdlib tables | none | no `stdlib/starlark.txt`; builtins counted external |
| 14. Confidence & certainty | partial | high via intra-file/imports, but issued on wrong-file and shadowed bindings |
| 15. Dynamic use & reflection | none | no dynuse; `getattr(...)()` result dropped |
| 16. Known gaps & limitations | n/a | documented in the rule `gap` text and the 10 verified gaps above |
