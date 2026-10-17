# cmake

The CMake plugin (`CmakePlugin`, id `"cmake"`, `crates/cgg-lang/src/plugins/cmake.rs`, 226 lines including tests) parses `.cmake` files with the crates.io grammar `tree-sitter-cmake` 0.7.1 (`Cargo.toml:118` declares `"0.7"`; `Cargo.lock` resolves 0.7.1; `crates/cgg-lang/Cargo.toml:57`). It is a small, single-pass walker: `function(...)`/`macro(...)` blocks become `FreeFunction` definitions, every `normal_command` becomes a bare call reference, and `include`/`add_subdirectory`/`find_package` are recorded as import records (`cmake.rs:1-7`, `56-173`). It declares no `signals()`, `shebangs()`, `max_line_bytes()` or `ts_language_for()` override, so all of those take the trait defaults (`crates/cgg-lang/src/lib.rs:298-324`). The `LanguagePlugin` trait has no `resolver_kind` method in this tree (grep of `crates/cgg-lang/src/lib.rs` finds none), so there is no per-plugin resolver kind to report. Resolution goes through the generic include arm of `cross_file::resolve`, which CMake shares with Verilog, VHDL, Erlang and Fortran (`crates/cgg-resolve/src/cross_file.rs:2123-2153`). For dead-code purposes the language is classed `script-driven` (`crates/cgg-resolve/src/deadcode/caps.rs:27-36`). It is registered at `crates/cgg-lang/src/plugins.rs:159`.

## 01. Detection & parsing

- **Extension `.cmake`**: the only extension the plugin declares (`cmake.rs:21-23`). The detector matches extensions case-sensitively first, then case-insensitively (`crates/cgg-lang/src/detect.rs:101-127`), so `.CMake` would also be detected (that follows from the code; not run).
  - Fixture `t1`: `helpers.cmake`, `main.cmake` and `cmake/util.cmake` were analyzed with `detected_via: extension:.cmake`.
- **`CMakeLists.txt` is NOT detected**: it has no `.cmake` extension and the plugin claims no filenames. Fixture `t1`: the audit logged `file_skipped ./CMakeLists.txt reason unknown-extension`, and the same for `./sub/CMakeLists.txt`. The framework rule's own `gap` text says so (`crates/cgg-core/src/frameworks/rules.rs:6675`: "the top-level CMakeLists.txt (which cgg does not analyze at all - only the .cmake extension is recognised)"). This is the biggest coverage hole: most CMake code lives in `CMakeLists.txt`.
  - `CMakeLists.txt` appears in `crates/cgg/src/rollup.rs:82`, but only as a project-root marker for `--rollup`, not as a detection rule.
- **Shebangs**: none. The trait default is `&[]` (`lib.rs:298-300`), and CMake scripts run as `cmake -P` carry no shebang anyway.
- **Content detection**: none. Content sniffing exists only for `.yaml`/`.yml`/`.json` and `.h` (`detect.rs:76-98`, `detect.rs:999`).
- **Grammar**: a single grammar, `tree_sitter_cmake::LANGUAGE` (`cmake.rs:24-26`). No dialects and no `ts_language_for` override.
- **Re-parse / fallback**: none. A file that fails to parse is still extracted from the error-recovered tree, with no retry (contrast the C# `#if` re-parse described in CLAUDE.md).
  - Fixture `t7` (an unbalanced `broken(` inside `function(user)`): the audit shows `parse_status: error`, and the definitions `user` and `after_error` that follow were **lost entirely**. Only `${name}`, `baz_fn`, `cfg_fn`, `find_fn` and `sub_fn` were extracted. With the line removed, `user` and `after_error` come back. One stray parenthesis can swallow the rest of the file.
- **Skip conditions**:
  - The depth gate is generic: `cgg_lang::exceeds_depth` against `MAX_TREE_DEPTH = 4000` produces `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155`, `451-454`). It applies to cmake like every other language.
  - Long lines: `max_line_bytes` is the default `None` (`lib.rs:310-312`), so cmake has no `LongLine` skip.
  - Minified: the minified-file rule (lines over 2,000 bytes) only gates the locals walk, and cmake records no locals (see 07). Not applicable.
- **Tests**: `cmake.rs:196` `function_macro_and_call` and `cmake.rs:211` `include_is_import`. `crates/cgg/tests/detect_prefixes.rs:87` writes `include(<prefix>)` into `.cmake` files to prove that every cmake `detect` prefix fires.

## 02. Callables extracted

- **`function(name args...) ... endfunction()`**: becomes a `DefVariant::FreeFunction` named by the first `argument` of the `function_command` header (`cmake.rs:58-61`, `90-124`). JSON kind is `function`; the audit kind is `freefunction`.
  - Example: `function(my_helper target)`.
  - Evidence: test `function_macro_and_call` (`cmake.rs:197`); fixture `t1` gives `my_helper`, `inner_util`, `unused_fn`, `driver` and others.
- **`macro(name args...) ... endmacro()`**: the same path with header `macro_command` (`cmake.rs:63-67`). It is also `FreeFunction`. Nothing marks it as a macro: no attribute, variant or `macro:` tag, so a CMake macro and a function cannot be told apart in the graph. Fixture `t1`: `noisy` is `kind: function`.
- **Nested definitions**: `function(inner_util t)  function(nested_def a) ... endfunction() ... endfunction()`. Both are extracted because the walker recurses into `function_def` children (`cmake.rs:59-60`). The nested one gets a bare qualified name (`nested_def`), with no owner prefix. Fixture `t1`: `inner_util -> nested_def` is a high intra-file edge.
- **Conditional definitions** (`if(WIN32) function(dup_fn) ... endfunction() endif()`): extracted as an ordinary second definition with the same name. No `DefRecord::conditional` flag is set (`cmake.rs:111-123` sets only name, lines, bytes and signature). Fixture `t1`/`t9`: two `dup_fn` nodes.
- **Qualified names and owners**: `qualified_name == simple_name ==` the raw first-argument text (`cmake.rs:111-113`). There is no namespace, file or owner component, so two files defining `foo` produce two nodes both called `foo`.
- **Quoted names**: `function("quoted_fn")` produces a node literally named `"quoted_fn"`, quotes included. The definition name is not unquoted, although import paths are (`cmake.rs:150-153`). Fixture `t2`: callable `"quoted_fn"@12`.
- **Variable-named definitions**: `function(${name})` produces a node literally named `${name}`. Fixture `t7`. Variable expansion is not attempted.
- **Signature hint**: `super::extract_signature(text)` stops at the first depth-0 `{` or at a `:` followed by a newline (`crates/cgg-lang/src/plugins.rs:74-86`). CMake has neither, so the "signature" is the whole collapsed body. Fixture `t1`: `driver` has `signature_hint: 'function(driver) noisy("hi") MY_HELPER(foo) … endfunction()'`.
- **Visibility and attributes**: always empty (`cmake.rs:120-121`).
- **Not extracted (none of these become nodes)**:
  - top-level script code: a file is not a callable (see 12 for how top-level calls are credited);
  - `block()`/`endblock()` scopes;
  - `cmake_language(EVAL CODE ...)` bodies;
  - `add_custom_command`/`add_custom_target`;
  - targets (`add_library`, `add_executable`) as nodes.
- **Not applicable**: methods, constructors, closures, properties, operators, overloads, abstract members. CMake has none of these.

## 03. Call-site forms

- **Bare command invocation** (`my_helper(mylib)`): each `normal_command` becomes a `RefRecord` with `name` = the identifier text and an empty `receiver_hint` (`cmake.rs:126-172`). Built-ins (`message`, `add_library`, `set`, …) are recorded the same way. Evidence: test `function_macro_and_call` asserts the refs `add_library` and `my_helper`.
- **Calls inside control flow** (`if`/`foreach`/`while`/`block`): recorded, because the walker recurses through every node (`cmake.rs:73-75`). Fixtures:
  - `t1`: `util_fn()` inside `foreach`/`if` bound (line 16).
  - `t2`: `only_here()` inside `block()` and `while()` was recorded at lines 6 and 9.
- **Case sensitivity: NOT handled**. CMake command names are case-insensitive, but the reference keeps its original case (`cmake.rs:134`), and only the import check lowercases (`cmake.rs:135-142`). Fixtures:
  - `t1`: `MY_HELPER(foo)` against `function(my_helper ...)` went to the external bucket (`no-candidate-in-file`, `name_screen_applied: external`).
  - `t2`: `Only_Here()` and `upper_macro()` (against `macro(Upper_Macro)`) were also external.
- **Calls in import position**: `include(...)`, `add_subdirectory(...)` and `find_package(...)` with a non-empty first argument become `ImportRecord`s and **no** reference (`cmake.rs:140-163`, early `return`).
- **Indirect calls: NOT supported**. All of these produce only a ref to the outer built-in, and the named function gets no edge:
  - `cmake_language(CALL fn ...)` and `cmake_language(DEFER CALL fn)`;
  - `cmake_language(CALL ${var})`;
  - `variable_watch(VAR fn)`.
  - Fixture `t1`: `deferred_fn` and `watcher_fn` have no inbound edge and are dead-code findings. `util_fn` was linked only by its separate direct call.
- **Not applicable**: method calls, qualified/path calls, chained calls, constructors, explicit generics, operators, paren-less calls. `RefRecord::arity` is never set (`cmake.rs:166-172` uses `..Default::default()`).
- **Value refs / registrar arguments**: none. The plugin never reads `ExtractCtx` (the parameter is `_ctx`, `cmake.rs:30`), so `--reference-edges` and registrar verbs have no effect on cmake.

## 04. Imports & module resolution

- **Import forms recorded** (`cmake.rs:140-163`). The kind is the lowercased command, and the path is the first argument with surrounding `"`/`'` trimmed:
  - `include(FetchContent)` gives kind `include` (test `include_is_import`, `cmake.rs:212`);
  - `add_subdirectory(deps/foo)` gives kind `add_subdirectory` (same test);
  - `find_package(Threads REQUIRED)` gives kind `find_package` (code path; fixtures `t1`/`t7`).
- **Path resolution** (`cross_file.rs:2123-2153`). For lang `cmake`, each import path is tried as a fixed list, each passed to `collect_include_defs` at depth 4:
  - `path`
  - `path.cmake`
  - `path.v`
  - `path.hrl`
  - `path.f90`
  - `path.f95`
  - the literal `CMakeLists.txt`
- **How a path is matched** (`cross_file.rs:3385-3445`). The path is first joined to the includer's directory and matched exactly. Failing that, the fallback takes files sharing the same last segment whose path `ends_with` the include path.
- **Results by form**:
  - `include(helpers)` → `helpers.cmake`: high (fixture `t1`, `driver -> noisy`, `cross-file:imports`).
  - `include(cmake/util)` → high (fixture `t3`).
  - `include(util)`, matched by suffix to `cmake/util.cmake` → high (fixture `t3`, `c -> util_fn`).
  - `find_package(Baz)` → `cmake/modules/Baz.cmake`: high (fixture `t7`).
  - `add_subdirectory(sub)` resolved in fixture `t7` only because `sub/sub.cmake` happens to match the suffix `sub.cmake`.
- **Forms that do not resolve**:
  - **The CMake `find_package` file conventions are not modelled.** `find_package(Foo)` does not try `FindFoo.cmake` and `find_package(Bar CONFIG)` does not try `BarConfig.cmake`/`bar-config.cmake`. Fixture `t7`: `find_fn` and `cfg_fn` bound only at medium through the step-1d global fallback.
  - **`add_subdirectory(dir)` does not look at `dir/CMakeLists.txt`.** The literal `CMakeLists.txt` try-path is joined to the *includer's* directory, and `CMakeLists.txt` is never analyzed anyway (section 01).
  - **Paths containing variables are unresolved**: `include(${CMAKE_CURRENT_LIST_DIR}/cmake/util.cmake)` and `include(${D}/...)` are not expanded. Fixture `t4`: the call stays `candidates-in-other-files`.
  - **`CMAKE_MODULE_PATH` is not modelled.** Only the suffix fallback stands in for it.
- **Transitive includes**: followed, but only for the raw path. The recursion (`cross_file.rs:3480-3490`) passes `imp.path` without the `.cmake` suffix list, and only for `imp.kind == "include"`. Fixture `t8`:
  - `a -include(b)-> b -include(c)-> c -include(e)-> e` did **not** reach `deep`;
  - the same chain with `include(c.cmake)`/`include(e.cmake)` bound `top -> deep` at high.
  - `add_subdirectory`/`find_package` imports inside an included file are not followed.
- **Import table semantics**: `collect_include_defs` pushes every definition of the target file into `direct_imports` as `simple_name -> qualified_name` (`cross_file.rs:3460-3470`). Step 1 then looks each qualified name up through `lookup_with_reexports(lang, qn, by_qn, ...)` (`cross_file.rs:5710-5753`).
- **BUG: an included name can bind to a same-named copy in a file that was not included.** cmake qualified names are bare, `by_qn` "keeps an arbitrary one" (`cross_file.rs:5724-5727`), and the C-family `c_include_copies` narrowing does not apply to cmake. In fixture `t3`, `main.cmake` does `include(cmake/util)` and calls `u3()`; `cmake/util.cmake` and `x/other.cmake` both define `u3`. The edge went to **`x/other.cmake` at `high`**. It did so again in `t3b`, with the copy moved to `zz/`.
- **Step 1d global fallback** (`cross_file.rs:6015-6055`). When a file has at least one resolved import, an otherwise-unbound bare name is bound to *any* same-named cmake callable in the tree, as long as there are 3 or fewer. These edges are medium. Fixtures:
  - `t6`: `include(helpers)` makes `util_fn()` bind to `cmake/util.cmake`, which was never included. `include(nonexistent)` does not, because it adds nothing to `direct_imports`.
  - `t8`: 4 candidates exceeds the ≤3 limit, so the site is `candidates-in-other-files`, not `fanout-cap-exceeded`.
- **No import, no cross-file edge.** A file with no resolved include gets no cross-file binding at all, even though CMake functions are global once defined. Fixture `t2`: `shared_name()` and `only_here()` in `c.cmake` stayed `candidates-in-other-files` (2 and 1 candidates).
- **`FileFacts::packages`**: never populated. Manifest reading (`manifest.rs`): none.
- **`heads.rs` external-module refusal / `WorkspaceVocab`**: cmake is not among the languages (Python/JS/TS/Go/Java/Kotlin/C#/C++ per CLAUDE.md), and grep of `heads.rs` finds no `cmake`. An include of a module outside the tree (`include(FetchContent)`) only fails to add names. Built-ins land in the external bucket by the generic "no candidate anywhere" screen (fixture `t1`: `name_screen_applied: external`).
- **`modules.rs` / `not-in-imported-module`**: not used for cmake (no grep hit). Not applicable.

## 05. Type inference & receiver typing

- Not applicable: CMake is untyped and has no receivers. The plugin never sets `receiver_hint`, `typed_receiver`, `arg_type`, `local_types`, `scoped_types` or `implicit_receiver` (`cmake.rs:166-172`). grep finds no `cmake` in `type_hints.rs`, `closure_types.rs` or `receiver.rs`.

## 06. Inheritance, dispatch & overloading

- Not applicable for the object-oriented parts: CMake has no classes, so there are no base records, `super`, `member_of_calling_class`, `overrides_below`, traits or extension methods.
- **List membership**: cmake is not in `OVERLOAD_ARITY_LANGS` or `INHERITANCE_LANGS` (per CLAUDE.md lists; grep finds no `cmake` in `dispatch.rs`), so `--dynamic-dispatch` adds nothing for cmake.
- **Redefinition**: CMake's runtime model is "last definition wins", and the previous one stays callable as `_name`. Neither is modelled:
  - Same file (fixture `t9`): two `dup` definitions → the call `dup()` is `ambiguous-in-file` and gets no edge; both definitions are reported dead.
  - Across an include (fixture `t1`): the call bound to one copy (the line-24 conditional copy) at high, picked by `by_qn`, not by any "last one wins" rule.
  - `_name()` calls to the overridden command are not recognised.

## 07. Scoping & shadowing

- **`FileFacts::locals`**: not populated. grep finds no `cmake` in `crates/cgg-lang/src/locals.rs`. So `shadowed-by-local` never fires for cmake.
- **Lexical narrowing of nested definitions**: cmake is not in the CLAUDE.md list (JS/TS, Lua, OCaml, F#, Scala, Kotlin, Swift, Rust, Dart, Haskell, Julia). Nested functions are global once defined in CMake anyway.
- **Intra-file linking**: generic `intra_file::link_file`. A use before the definition in the same file binds. Fixture `t2`: `late_user -> later_def` at high.
- **Conditional definitions**: no `DefRecord::conditional` (that is PHP-only). The two copies compete as ambiguous (fixture `t9`).
- **Variable scope** (`set(... PARENT_SCOPE)`, `block(SCOPE_FOR ...)`): not modelled. Not applicable to the call graph.

## 08. Macros, preprocessor & codegen

- **CMake `macro()`**: extracted as an ordinary `FreeFunction` (section 02). It gets no `macro:<name>` tag and no expansion. The `macro-generated` cap does not apply.
- **`#define`/`#if`, templates, quote/unquote**: not applicable.
- **`name_mentions`**: never populated by the cmake plugin (no write to it in `cmake.rs`). Function names in string arguments (`cmake_language(CALL fn)`, `variable_watch(VAR fn)`, `set_property(... fn)`) are therefore not even recorded as doubt for the dead-code pass (fixture `t1`: `deferred_fn` and `watcher_fn` are findings with evidence `no-unresolved-site-with-name`).
- **Generated code** (`configure_file`, `file(WRITE ...)`, `cmake_language(EVAL CODE ...)`): not modelled. The rule `gap` text names `EVAL CODE` explicitly (`rules.rs:6675`).

## 09. FFI & cross-language links

- `ffi::link_ffi`: no cmake coverage (grep finds no `cmake` in `crates/cgg-resolve/src/ffi.rs`).
- **No CMake → C/C++ link.** `add_library(foo foo.c)`, `target_sources` and `add_executable` are recorded only as external built-in calls; the sources they name are not linked.
- **No CMake → script link.** `add_custom_command(COMMAND ${CMAKE_COMMAND} -P script.cmake)` is not linked to `script.cmake`.
- Descriptor links: not applicable. Markup code-behind: not applicable.

## 10. Frameworks & entry points

- **Rule `cmake`** (`rules.rs:6629-6678`): `language: "cmake"`, `kind: TrustKind::Lifecycle`, `node: true`. The only rule whose language is cmake (grep `language: "cmake"`; no multi-language rule lists cmake).
  - `detect` (32 import prefixes, all matched against `include`/`find_package`/`add_subdirectory` paths):
    - module include names: `FetchContent`, `ExternalProject`, `ExternalData`, `CTest`, `CPack`, `GNUInstallDirs`, `CMakePackageConfigHelpers`, `CMakeParseArguments`, `CMakeDependentOption`, `CMakeFindDependencyMacro`, `FindPackageHandleStandardArgs`, `FeatureSummary`, `GoogleTest`, `InstallRequiredSystemLibraries`;
    - check modules: `CheckIncludeFile`, `CheckIncludeFiles`, `CheckSymbolExists`, `CheckFunctionExists`, `CheckLibraryExists`, `CheckCSourceCompiles`, `CheckCXXSourceCompiles`, `CheckSourceCompiles`, `CMakePushCheckState`, `TestBigEndian`;
    - `find_package` names: `PkgConfig`, `Threads`, `Boost`, `OpenSSL`, `ZLIB`, `Python3`, `Doxygen`, `Git`.
  - Everything else is empty: `detect_paths`, `detect_calls`, `attributes`, `registrars`, `base_types` and `methods` are all `NONE`, and `string_targets` is `false`.
  - It is a **detect-only** rule: detection runs through `prefix_index` (`crates/cgg-resolve/src/frameworks/mod.rs:204-211`, `262-269`), and the only effect is a "seen, no rules" disclosure carrying the `gap` text.
  - Fixture `t1` (`include(FetchContent)`, `find_package(Threads)`): `seen, no rules cmake — found in 1 file(s), entries NOT enumerated`, `nodes_minted: 0`.
  - Test `every_detect_prefix_can_actually_fire` (`crates/cgg/tests/detect_prefixes.rs:149`, cmake form at `:87`).
- **Entry nodes**: none are ever minted for cmake. The `gap` text explains why: the entry is the file itself, run as `CMakeLists.txt`, `cmake -P`, `ctest -S`, a toolchain file or `Find<Pkg>.cmake`/`<Pkg>Config.cmake`, or a callable reached through `cmake_language(DEFER CALL ...)`.
- The remaining rule tables have no cmake entries (nothing in `rules.rs` names cmake outside the rule above):
  - `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`: none.
  - Visibility entries: none (no visibility is extracted).
  - Registrar verbs: none (the plugin ignores `ExtractCtx`).
- **Benchmark corpus**: `scripts/benchmark.sh:95` (`cmake-kitware`, subdir `Modules`). README benchmark row: `CMake/Modules | cmake | 946 callables | 866 edges | 9% cross-file | 376ms` (`README.md:1347`). The `APPS` line at `benchmark.sh:355` lists the C++/other rules seen in the Kitware repo, not cmake ones.

## 11. Test harness entries

- None. No rule of kind `test` covers cmake: CTest's `add_test(...)` registrations and `ctest -S` scripts are not recognised. `CTest` is only a detect prefix of the Lifecycle rule above. `TEST_CLASS_MEMBERS_ARE_ROOTS` does not apply (no classes). No test-file gating, and the plugin sets no `TestRole` (`test_defs` signal false: `capabilities.test_tagging: none` in fixture `t1`'s dead-code JSON).

## 12. Dead-code analysis

- **Language class `script-driven`**: `SCRIPT_DRIVEN` contains `"cmake"` (`caps.rs:27-36`, applied at `caps.rs:165`; test `script_languages_are_classified_apart`, `caps.rs:250`, exercises the bash member).
  - **Confidence cap**: `Evidence::LanguageIsScriptDriven` caps a finding at **low** (`crates/cgg-core/src/deadcode.rs:314-316`). Root-dependent categories get `SuppressionReason::ScriptDriven` (`crates/cgg-resolve/src/deadcode/mod.rs:811-812`), so no cmake finding is ever shown at the default high threshold.
  - Fixture `t1`: `findings 0 shown (high) · 4 withheld (low)`.
  - Capabilities row: `class script-driven, visibility none, attributes none, value_references none, dispatch none, exports none, test_tagging none, max_confidence medium, root_rules_active [toplevel:invocation]`.
- **Roots**:
  - **`toplevel:invocation`** (`crates/cgg-resolve/src/deadcode/roots.rs:550-612`). A top-level call is recorded as `no-enclosing-callable`, and every callable of the same `(language, simple_name)` *anywhere in the tree* is rooted. Fixture `t1`: `driver()` at `main.cmake:23` rooted `driver`, and 7 of 11 callables were reachable (63%). Fixture `t9`: `user()` rooted `user`.
  - **Case limitation**: the match is case-sensitive. Fixture `t2`: top-level `upper_macro()` against `macro(Upper_Macro)` went to the external bucket rather than `no-enclosing-callable`, so `Upper_Macro` was not rooted.
  - Calls in `CMakeLists.txt` never root anything, because the file is not analyzed. In a typical project every function used only from `CMakeLists.txt` therefore reads as unreferenced.
- **Evidence attached to a typical finding** (fixture `t1`, `deferred_fn`): `language-is-script-driven`, `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges`, `no-unresolved-site-with-name`.
- **No cmake-specific signal is extracted**: no visibility, attributes, exports, value refs, `dyn_uses`, unreachable code or impls. `signals()` is the default (`lib.rs:304-306`).
- **Caps that never apply**: `implements-external-contract`, `macro-generated` and `reflection`.
- **Unresolved-doubt handling**: generic. An unresolved same-name site counts as doubt. Function names that appear only as *arguments* (`cmake_language(CALL f)`) are not sites, so they add no doubt (fixture `t1`).

## 13. Builtins & stdlib tables

- **No `stdlib/cmake.txt`**: `crates/cgg-core/src/stdlib/` holds 30 tables and none is cmake, and `stdlib_names` has no `"cmake"` arm (`crates/cgg-core/src/stdlib.rs:221-258`).
- **Built-in commands go to the external bucket.** `message`, `set`, `add_library`, `cmake_language`, `get_filename_component` and the rest are counted as **external**, not stdlib. Fixture `t1` metrics: `stdlib_calls: 0, external_calls: 12`, and every built-in carries `name_screen_applied: external`.
  - `--include-stdlib` therefore adds nothing for cmake.
  - `--include-external` mints `<external>::message`, `<external>::add_library`, `<external>::cmake_language` and so on (fixture `t1`). Case-mismatched user calls show up there too (`<external>::MY_HELPER`).
- **Consequence for step 1d**: the global fallback skips names found in `stdlib_names` (`cross_file.rs:6022-6023`). With no cmake table, a user function sharing a built-in's name is never protected or disambiguated.
- `rust_types`-like tables, `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, implicit packages: none, not applicable.

## 14. Confidence & certainty

- **High**:
  - **intra-file bare call to a unique same-file definition**: fixtures `t1` (`my_helper -> inner_util`, `inner_util -> nested_def`) and `t2` (`late_user -> later_def`).
  - **cross-file call through a resolved `include`/`find_package` path** (`cross-file:imports`): fixtures `t1` (`driver -> noisy`), `t3` and `t8`.
  - **High can be wrong.** The `t3` `u3` edge was high while pointing at a file that was not included (section 04).
- **Medium**: only the step-1d global by-simple fallback (fixture `t1` `util_fn`, `t6`, `t7` `find_fn`/`cfg_fn`). Mermaid marks these with `?` (fixture `t1`: `N10 -->|?| N0`).
- **Low**: never produced for edges. `--guess-unique-names` is Rust-only.
- **Refusals seen**:
  - `ambiguous-in-file`: same-file redefinition (fixture `t9`).
  - `candidates-in-other-files`: no import, or more than 3 candidates (fixtures `t2`, `t8`).
  - `no-candidate-in-file`: names found nowhere, external (fixture `t1`).
  - `no-enclosing-callable`: top-level calls (fixture `t1`).

## 15. Dynamic use & reflection

- **dynuse**: none. cmake is not among dynuse's languages (Python, JS, Java, C#, Ruby, PHP per CLAUDE.md), and grep finds no `cmake` in `plugins/dynuse.rs`.
- **Not handled** (each yields only a call to the outer built-in, no edge, no doubt):
  - `cmake_language(CALL <name>)`, `cmake_language(DEFER CALL <name>)`, `cmake_language(EVAL CODE "...")`;
  - `variable_watch(<var> <command>)`;
  - commands named through variables (`${fn}`).
  - Fixture `t1`.

## 16. Known gaps & limitations

- **CMakeLists.txt is not analyzed** (fixture `t1`; `rules.rs:6675`). Most real CMake code and every top-level invocation live there, so CMake coverage is mostly `.cmake` modules (README benchmark uses `Modules/`).
- **Command-name case sensitivity**: CMake is case-insensitive, cgg is not (`cmake.rs:134`; fixtures `t1` `MY_HELPER`, `t2` `Only_Here`/`upper_macro`). This loses both edges and top-level roots.
- **An include binds to an arbitrary same-named copy at high**: bare qualified names plus `by_qn` arbitrary pick (`cross_file.rs:5724-5753`; fixtures `t3`/`t3b`). The edge points at a file that was not included.
- **No global function namespace**: a file with no resolved include gets no cross-file edges (fixture `t2`). Any one resolved include enables an unrelated global ≤3 fallback at medium (fixture `t6`). Both behaviours are inconsistent with CMake's model, where any previously defined function is visible.
- **Transitive includes need the explicit `.cmake` suffix** (`cross_file.rs:3480-3490`; fixture `t8`). `add_subdirectory`/`find_package` are not followed transitively.
- **`find_package`'s `Find<Pkg>.cmake` / `<Pkg>Config.cmake` / `<pkg>-config.cmake` conventions are not modelled** (`cross_file.rs:2133-2141`; fixture `t7`). `add_subdirectory(dir)` does not reach `dir/CMakeLists.txt`.
- **Variable-bearing paths and names are unexpanded**: `include(${CMAKE_CURRENT_LIST_DIR}/x.cmake)` (fixture `t4`), `function(${name})` (fixture `t7`).
- **Quoted function names keep their quotes**: `"quoted_fn"` (fixture `t2`).
- **Parse errors silently drop every later definition in the file**, with no re-parse (fixture `t7`).
- **Indirect invocation is invisible**: `cmake_language(CALL|DEFER CALL|EVAL CODE)`, `variable_watch` (fixture `t1`). Names in string arguments are not even recorded as `name_mentions`.
- **No distinction between `macro()` and `function()`**: both are `FreeFunction` with no tag (`cmake.rs:58-67`).
- **Redefinition and override semantics** (last-wins, `_name` for the previous definition) are not modelled (fixture `t9`).
- **`signature_hint` is the whole body**: `extract_signature` finds no `{` (fixture `t1`).
- **No stdlib table**: built-ins inflate the external bucket (section 13).
- **No test harness rule**: CTest `add_test` is not recognised.
- **No CMake → C/C++ source link**: no FFI-like linking of targets.
- **CHANGELOG**: no entry targets the CMake plugin. The `cmake` mentions in `CHANGELOG.md` (lines 1441, 1534, 1564, 2074, 2462, 3847, 3934) are about the `cmake-kitware` corpus repository's C/C++ code or its timing.
- No ignored tests: the two plugin tests are plain `#[test]` (`cmake.rs:196`, `211`).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.cmake` only; `CMakeLists.txt` skipped as unknown-extension; no re-parse, parse errors drop later defs |
| 02 Callables extracted | partial | `function()`/`macro()` (incl. nested) as bare-named FreeFunction; quotes/`${}` kept verbatim; macro untagged |
| 03 Call-site forms | partial | every `normal_command` as bare call; case-sensitive; `cmake_language(CALL)`/`variable_watch` invisible |
| 04 Imports & module resolution | partial | include/add_subdirectory/find_package via generic include arm; no Find/Config conventions, no `${}` paths, transitive needs `.cmake`; wrong-copy high binding bug |
| 05 Type inference & receiver typing | n/a | untyped, no receivers |
| 06 Inheritance, dispatch & overloading | n/a | no OO; redefinition yields ambiguous-in-file; `_name` override not modelled |
| 07 Scoping & shadowing | minimal | no locals; generic intra-file only |
| 08 Macros, preprocessor & codegen | none | `macro()` not tagged; no name_mentions; EVAL CODE/configure_file unmodelled |
| 09 FFI & cross-language links | none | no CMake→C/C++ or `-P` script links |
| 10 Frameworks & entry points | minimal | one detect-only Lifecycle rule `cmake` (32 prefixes) that discloses a gap; no entry nodes |
| 11 Test harness entries | none | CTest `add_test` not recognised |
| 12 Dead-code analysis | minimal | script-driven class: findings capped low/withheld; only `toplevel:invocation` roots (case-sensitive) |
| 13 Builtins & stdlib tables | none | no `stdlib/cmake.txt`; built-ins counted as external |
| 14 Confidence & certainty | partial | high for intra-file and resolved includes (can be wrong), medium only via step-1d global fallback |
| 15 Dynamic use & reflection | none | no dynuse; `cmake_language`/`variable_watch` ignored |
| 16 Known gaps & limitations | n/a | 16 gaps listed above, the worst being CMakeLists.txt skipping, case sensitivity and wrong-copy high edges |
