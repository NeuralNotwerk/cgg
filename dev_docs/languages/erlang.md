# erlang

The Erlang plugin (`ErlangPlugin`, `crates/cgg-lang/src/plugins/erlang.rs`, 325 lines including tests) is one small recursive walker over the `tree-sitter-erlang` 0.16 grammar, taken from crates.io (`Cargo.toml:108`, `crates/cgg-lang/Cargo.toml:48`). It is registered at `crates/cgg-lang/src/plugins.rs:150`. It records five things: `-module`, `-import`, `-include`/`-include_lib`, `-behaviour`/`-behavior`, every `function_clause` as a definition, and every `call` node as a reference with its arity. It has no `signals()` override, so all of `PluginSignals` is false (`crates/cgg-lang/src/lib.rs:304`, default). It reads no locals, visibility, exports, attributes, value references or dynamic uses. The `LanguagePlugin` trait has no `resolver_kind` method in this tree. Resolution is driven by language-id checks spread across the resolver:

- the clause-language rules in `crates/cgg-resolve/src/intra_file.rs:859`
- the remote-call `m:f` lookup in `crates/cgg-resolve/src/cross_file.rs:5569`
- the import/include arms in `cross_file.rs:2064` and `cross_file.rs:2129`
- the type-inference opt-out in `crates/cgg-resolve/src/type_hints.rs:389`

Verification: the fixtures cited below are under `<scratch>/parity/erlang/` (f1–f11). Each one was run with `target/release/cgg` 0.11.0 and `-t json -o`, and the edges were dumped with `dump.py`.

## 01. Detection & parsing

- **Extensions `.erl`, `.hrl`.** These are the plugin's `extensions()` (`erlang.rs:15-17`). Fixture f1: `src/alpha.erl` shows `detected_via: extension:.erl` and `include/defs.hrl` shows `extension:.hrl`.
- **Shebang `escript`.** This is `shebangs()` (`erlang.rs:18-20`). Fixture f1: an extension-less `script` starting `#!/usr/bin/env escript` was analyzed (`detected_via: shebang:escript`), and its `main` → `go` edge resolved `high`.
- **`.escript` extension: NOT detected.** Fixture f8: `tool.escript` without a shebang was skipped as `unknown-extension`.
- **Content detection: none.** There is no Erlang sniff in `crates/cgg-lang/src/detect.rs`. A grep for `erl` there returns nothing.
- **OTP metadata files are not parsed.** `.app.src`, `.app`, `rebar.config`, `sys.config` and `.config` were all skipped as `unknown-extension` in fixture f8. So the `{mod, {M, Args}}` application entry tuple is invisible, a gap that rule `erlang-application` names itself (`rules.rs:5833`).
- **Grammar.** One grammar, `tree_sitter_erlang::LANGUAGE` (`erlang.rs:21-23`). There is no per-file dialect (`ts_language_for` keeps its default).
- **Re-parse/fallback: none.** A file with a syntax error is extracted from the error-recovered tree, and its audit shows `parse_status: "error"` (`crates/cgg/src/lib.rs:457`). Fixture f8/broken.erl: the clause after `bad( -> .` (`after_bad/0`) was lost as a definition. Its call `fine()` was then recorded as a top-level site with no enclosing callable (`no-enclosing-callable`), which made `fine` a top-level root in dead-code.
- **Skip conditions (generic, not Erlang-specific).**
  - Depth: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `TooDeep` (`crates/cgg/src/lib.rs:155,451`).
  - Minified: `MINIFIED_LINE_BYTES` = 2000 (`lib.rs:159`) only gates locals collection, which Erlang does not produce anyway, plus `--skip-minified` in the walker (`crates/cgg-walk/src/lib.rs:133`).
  - Long lines: no `max_line_bytes` override, so no long-line cap.
- **Framework-detection prefix test.** `crates/cgg/tests/detect_prefixes.rs:69` writes `-module(m).\n-behaviour(<prefix>).` for every Erlang rule and asserts that each rule is detected.

## 02. Callables extracted

- **Named function clauses.** `f(X) -> ...;` and `f(Y) -> ...` become one `DefRecord` **per clause** (`DefVariant::FreeFunction`), with no visibility and no attributes. The walker matches `"function_clause" | "function"` and takes `child(0)` when it is an `atom` (`erlang.rs:104-107,191-217`). Test `module_and_functions` (`erlang.rs:292`). Fixture f7: `f(0) -> zero; f(N) -> N.` produced two nodes, `k:f@4` and `k:f@5`.
- **Qualified name `module:name`.** The module comes from the first `atom` child of `-module(...)` (`erlang.rs:128-139`), and the name is formed in `qn()` (`erlang.rs:54-60`). Example: `alpha:helper`.
- **Files without `-module` get bare names.** This applies to `.hrl` headers and escripts. Fixture f1: `hdr_fun` (from `defs.hrl`) and `main`, `go` (from the escript). Fixture f6/nomod.erl: `helper`, `user`.
- **Arity is not part of the qualified name.** `f/1` and `f/2` share `m:f`. The arity is recovered later from `signature_hint` by `clause_arity` (`intra_file.rs` test `clause_arity_reads_the_signature`). `stable_ids.rs:217` notes that Erlang has overloads "in quantity", which is why names are not node ids.
- **`signature_hint` holds the whole clause text.** It is `super::extract_signature(self.text(node))` (`erlang.rs:211`). Fixture f6: the dead-code JSON's `signature_hint` for `m:go` holds the entire body, truncated.
- **Owner/kind.** Every definition is kind `function`. There are no owners, classes or methods.
- **NOT extracted:**
  - Anonymous funs `fun(X) -> ... end` and named funs `fun Self(0) -> 1; Self(K) -> ... end` (`anonymous_fun`/`fun_clause` nodes are never matched). Calls inside them are attributed to the enclosing clause: fixture f1, `spawn(fun() -> helper(X) end)` gives `alpha:run → alpha:helper` at L34.
  - `-spec`, `-callback`, `-type`, `-opaque`, `-record` declarations (fixture f6: no node for `-callback cb/1` or `-spec go/1`).
  - Functions generated by macros (`-define(MKFUN(N), N() -> ok). ?MKFUN(gen1).`): no node, fixture f8.
  - A multi-clause function as a single node. Each clause is its own node, so mermaid shows `alpha:handle_call` twice (fixture f1, N5/N6).
- **Conditional compilation.** Both `-ifdef`/`-else` branches are extracted, giving two `p:f` nodes (fixture f8). See §07.

## 03. Call-site forms

- **Bare local call `f(X)`.** Recorded as name `f` with an empty receiver and `arity = Some(named args)` (`erlang.rs:254-264`). Fixture f1: `local_helper(X)` → `alpha:local_helper`, `high`, `intra-file`.
- **Remote call `m:f(X)`.** The grammar nests it as `remote(module, fun: call)`. The plugin rewrites the name to `m:f` with `receiver_hint = m` (`erlang.rs:227-252`). Test `a_remote_call_records_its_module_as_the_receiver` (`erlang.rs:302`). Fixture f2: `b:g(X, X)` → `b:g@3` (g/2) `high`, and `b:g(X)` → `b:g@4` (g/1) `high`.
- **Own-module remote call `m:f()` inside module `m`.** Binds in-file by qualified name (`intra_file.rs:254`). Test `erlang_remote_call_binds_in_file_only_for_its_own_module`.
- **`?MODULE:f(X)` / `?SERVER:f()`.** The module is a `macro_call_expr`, not an atom, so the site is recorded as a bare `f`. It binds the local `f` (fixture f1 L25, fixture f6 L32, both `high`). It is correct only when the local function exists.
- **Variable-module call `Mod:f(X)` or `(expr):f(X)`.** Also recorded as a bare `f`. **Wrong binding:**
  - Fixture f10: `t(Mod) -> Mod:handle(1)` was bound to the file's own `a:handle@3` at **high**, although `Mod` is a runtime value.
  - Fixture f1 L36: `Mod:bfun(X)` was bound through `-import(beta, …)` to `beta:bfun/2` at medium.
- **Calling a fun variable `F(X)`.** Recorded as a bare call named `F`. With no locals it never binds, and is classified **external** (`<external>::F` with `--include-external`, fixture f1 L31 and f6 `Fact`/`G`/`H`/`Self`). It is not refused as `shadowed-by-local`.
- **Arity is recorded on every call site** (`erlang.rs:255-257`). It is used for clause narrowing (§06).
- **Guard BIFs (`when is_integer(N)`).** Recorded as ordinary calls and screened as stdlib (fixture f6: `<stdlib>::is_integer`).
- **Calls inside `case`/`receive`/`after`/`try`/`catch`/list comprehensions/records/`begin…end`.** All recorded (fixture f6: `c0` … `c6`, `c10`, `c11` bound `high`).
- **Nested call as argument `c14(c15(1))`.** Both recorded (fixture f6).
- **NOT recorded:**
  - Function references `fun f/1`, `fun m:f/1`, `fun ?MODULE:f/0` (`internal_fun`/`external_fun` nodes are never visited as references). Fixture f6: `c7`, `c8` and `c13` (passed as `c12(fun c13/0)`) got no edge and were reported dead.
  - Macro invocations `?DOUBLE(X)` and `?CALL(1)` (`macro_call_expr` is not `call`). The call inside the *definition* `-define(CALL(X), target(X))` is recorded as a top-level site (§08).
  - MFA triples `spawn(m, c9, [])`, `apply(M, F, A)`, `erlang:apply(alpha, helper, [X])`, `gen_server:call(?MODULE, …)` → `handle_call`. Only the outer BIF/stdlib call is seen (fixture f1 L32/L33, fixture f6 `c9` dead).
  - Value refs, callbacks or registrar arguments. There is no registrar capture (no Erlang `registrars` in any rule, so `registrar_verbs_for("erlang")` returns `None`, `rules.rs` around line 11935).
  - Constructors, explicit generics, operators and paren-less calls: not applicable to Erlang.
  - Erlang → Elixir calls `'Elixir.Foo':bar(1)`. The module is a quoted atom and the site goes to external (fixture f9).

## 04. Imports & module resolution

- **`-import(Mod, [f/1, …])`.** Recorded as an `ImportRecord{kind:"import", path: Mod}` (`erlang.rs:141-162`). **The function list is discarded.** The resolver treats the module as an unqualified prefix and a module alias (`cross_file.rs:2064-2077`), so any bare call can bind into `Mod` by name, **arity-blind**:
  - Fixture f3: `-import(b,[g/1])`, then `g(X)` → `b:g@4` (g/3, the last clause) at medium, and `g(X, X)` → the same g/3.
  - Fixture f2: `h(X)` (not in the import list) → `b:h` at medium.
  - Fixture f1 L24: `bfun(Y)` → `beta:bfun/2`, the wrong arity.
- **`-include("x.hrl")`.** Recorded as `kind:"include"` (`erlang.rs:75-94`). It is resolved by the include arm, which tries `path` and `path.hrl` (`cross_file.rs:2124-2140`, `lang == "erlang"`). Fixture f1: `hdr_fun(X)` → `hdr_fun` in `include/defs.hrl`, `high`, `cross-file:imports`. Fixture f11: `-include("y.hrl")` from `app/src/` reached `app/include/y.hrl` at high.
- **`-include_lib("app/include/x.hrl")`.** Recorded as `kind:"include_lib"` (test `include_lib_is_recorded`, `erlang.rs:313`). There is no dedicated resolver arm (grepping `cross_file.rs` for `"include_lib"` finds nothing). Fixture f11: `lib_hdr()` still bound at **medium** through the by-name fallback. Its main use is framework detection (eunit, common_test).
- **`-behaviour(B)` / `-behavior(B)`.** Recorded as `kind:"behaviour"` (`erlang.rs:164-189`). It is used only for framework detection (§10). It is not a base class.
- **Remote module resolution `m:f`.** `cross_file.rs:5566-5600` (FINDINGS R14):
  - Only callables whose qualified name equals `m:f` are taken, narrowed by `narrow_copies` toward the caller's file/directory, then by `first_clause_per_arity`.
  - An unknown module or function returns `None` and ends **external**. Fixture f2: `b:nosuch(X)` → `<external>::b::b:nosuch`, never another module's `nosuch`.
  - Tests: `erlang_remote_call_binds_its_module_only` and `copies_are_files_and_test_data_is_not_shipped` (`cross_file.rs:~13481,~14300`), and `copies_of_a_name_need_evidence` (`cross_file.rs:~12717`, where `-import(lists, …)` is not `lists1`).
- **Copies of one module.** Fixture f2 (`c.erl` and `v2/c.erl` both `-module(c)`): `c:k(X)` bound once, `high`, to the copy nearest the caller. CHANGELOG notes that copies of the module a call names prefer the non-test-data copy (CHANGELOG.md:478-482).
- **Bare call without an import.** It never binds into another module. Fixture f4 (two candidates) and fixture f5 (one candidate) both stay unresolved as `candidates-in-other-files`. This is correct Erlang semantics.
- **`FileFacts::packages`: not populated.** **Manifest reading** (`rebar.config`, `.app.src`): none.
- **External-module refusal (`heads.rs` `WorkspaceVocab`): Erlang is not covered** (`heads.rs` contains no `erlang`). The effect is reached instead by the `m:f` exact lookup returning nothing plus `classify_external` (`crates/cgg-core/src/external.rs:144-200`). Unknown modules land in the external bucket, reported as `(unattributed)` in `unresolved_by_module` (fixture f1 audit).
- **`not-in-imported-module`: not used for Erlang.** A known module lacking `f` goes external, not refused (fixture f2).
- **Elixir → Erlang.** `:mod.f()` in Elixir is resolved against Erlang callables through `LibIndex::resolve_elixir` (`libscope.rs:206-209,1014-1025,1091-1100`). Fixture f9: `:b.g(x)` → `b:g@2`, high. `:b.nope(1)` (module in the tree, function missing) appeared as `<external>:::b::nope` with `--include-external`, although the code returns `Scoped::Refused` for a known module. That is the observed output; how the refusal maps to external is unverified.

## 05. Type inference & receiver typing

- **Not applicable / deliberately disabled.** `propagate_types_with_fields` returns immediately for Erlang (`type_hints.rs:386-391`). The comment there reads: "An Erlang receiver is the module atom of `m:f(…)` — never a variable". Test `erlang_module_receivers_are_never_typed` (`type_hints.rs:3104`). CHANGELOG.md:530 says a `file` receiver had become a `File`.
- Not used for Erlang:
  - parameter, local, field or return typing (`build_return_type_map` sees Erlang defs, but no Erlang receiver is rewritten)
  - `declared_type`
  - casts and constructions (`CAST_INTRINSICS`/`CONSTRUCTING_METHODS`)
  - closure parameter typing (`scoped_types`)
  - implicit receivers
  - `weak_receiver_type`
- `-spec` type signatures are not read at all.
- Erlang is listed in the `lang_tallies_table!` profiling macro (`type_hints.rs:337`). That is a counter, not a typing feature.

## 06. Inheritance, dispatch & overloading

- **Clause language.** `is_clause_language` covers `"erlang" | "elixir" | "haskell"` (`intra_file.rs:858-860`). Same-name definitions are treated as the clauses of one function per arity, not as overloads to guess between.
- **Arity narrowing.** `first_clause_per_arity` (`intra_file.rs:941-985`) keeps the candidates whose `clause_arity(signature_hint)` equals the call's arity, then the first clause per (file, arity). A single result is `high`.
  - Test `erlang_clauses_bind_once_to_the_function_of_the_calls_arity` (`intra_file.rs:~1366`).
  - Fixture f7: `f(0)` → `k:f@4` high, `f(1, 2)` → `k:f@6` high, `k:f(5)` → `k:f@4` high.
- **Arity mismatch falls back to every arity.** When no candidate has the call's arity, all arities stay ("if that leaves nothing … every function stays", `intra_file.rs:862-865`):
  - Fixture f7: `f(1, 2, 3)` with only f/1 and f/2 bound to **both** at medium. With `--fanout-cap 0` it became `fanout-cap-exceeded {candidates: 2}`.
  - Fixture f2: `a:t(X, X)` with only `t/1` bound to `t/1` at **high**, a false high.
- **Clause-language arity sets stay capped.** They are not exempt like `one_method` overload sets (CLAUDE.md, cross_file step 3). Erlang is not in `OVERLOAD_ARITY_LANGS` (`type_hints.rs:1583`: java, scala, cpp only).
- **Arity narrowing is not applied on the `-import` path** (fixture f3, §04).
- **Base classes, super/base, `member_of_calling_class`, `overrides_below`, interface/trait impls, extension methods, mixins: not applicable.** Erlang has no classes.
- **`-behaviour` is not modelled as an interface/implementation pair.** There is no `trait_impl_target`, so `--dynamic-dispatch` adds nothing. Erlang is not in `INHERITANCE_LANGS` (`dispatch.rs:110`). Fixture f7 with `--dynamic-dispatch --reference-edges` produced the same edge set.
- **The `-callback` declarations of a behaviour module are not extracted** (§02), so callbacks cannot be linked to their behaviour.

## 07. Scoping & shadowing

- **`FileFacts::locals`: not produced.** `crates/cgg-lang/src/locals.rs` has no Erlang node kinds (grep finds none). So:
  - `shadowed-by-local` never fires.
  - A call through a fun-valued variable (`F(X)`) is classified external rather than refused (fixture f1, f6).
  - Erlang variables are capitalised and atoms are lower-case, so a variable cannot shadow a function name. Missing locals therefore cause no wrong binding, only external noise.
- **Lexical narrowing of nested definitions: n/a.** Erlang has no nested named functions, and named funs are not extracted. Erlang is not in the lexical-narrowing list in CLAUDE.md.
- **Positional shadowing: n/a.**
- **Conditional definitions (`-ifdef`/`-else`/`-endif`).** Every branch is walked. Both definitions become clauses of one function, and a call binds only to the **first** (fixture f8: `t() -> f()` → `p:f@3` only). The `-else` branch's `p:f@5` and everything only it calls (`p:b`) are then unreachable. No `DefRecord::conditional` flag is set.

## 08. Macros, preprocessor & codegen

- **No macro expansion.** `-define` is not expanded. `?NAME(...)` invocations (`macro_call_expr`) are not recorded as calls (fixture f6 `?CALL(1)`, fixture f1 `?DOUBLE(X)`). The arguments of a macro call are still walked, so a real call nested in them would be recorded.
- **Calls in macro bodies are recorded as top-level sites.** The walker descends into `pp_define`, so `-define(CALL(X), target(X))` records `target(X)` with no enclosing callable. That makes `m:target` a `toplevel:invocation` dead-code root (fixture f6 dead-code JSON `roots`). The result is accidental but approximately right: the function is live if the macro is used, and wrongly live if it is not.
- **Macro-generated functions** (`?MKFUN(gen1).`) are not created. A call to `gen1()` falls to external (fixture f8, `<external>::gen1`).
- **No `macro:<name>` tagging, no `name_mentions`, no `fn_macros`.** The C/Rust macro machinery (`c_macro.rs`, `rust_macro.rs`) does not apply.
- **`-ifdef`/`-if`/`-else`:** all branches extracted (§07).
- **Parse transforms** (`-compile({parse_transform, M})`) and EUnit's `?_test`/`?_assert` generators: not handled. The eunit rule's `gap` text names this (`rules.rs:5901`).
- **Records** (`#r{}`): the field expressions' calls are recorded (fixture f6 `c10`). The record itself is not a node.

## 09. FFI & cross-language links

- **NIFs (`erlang:load_nif/2`, `-nifs([...])`) and ports: none.** `crates/cgg-resolve/src/ffi.rs` has no NIF, port or BEAM handling (grepping for `nif`, `beam` or `port_` finds nothing relevant). Erlang stubs that are replaced by C NIFs (`enif_*` in C) are not linked.
- **Not applicable / not covered:** PyO3, wasm-bindgen, napi, JNI, P/Invoke, `extern "C"`, asm.
- **Elixir ↔ Erlang** is the only cross-language link:
  - Elixir `:mod.f()` → Erlang `mod:f` (`libscope.rs:1091`, fixture f9, `high`).
  - Erlang → Elixir (`'Elixir.Foo':bar()`) is not resolved (fixture f9, external).
- **Descriptor links (protobuf rpc → host): no Erlang host.** No Erlang-specific handling was found in `descriptor.rs` (grep finds no `erlang`). Unverified whether a gpb-generated module would ever be linked.
- **Markup code-behind: n/a.**

## 10. Frameworks & entry points

Detection uses the file's import records (`import`, `include_lib`, `behaviour` kinds). Every rule below has `language: "erlang"`, no `attributes`, no `registrars`, no `base_types`, no `detect_paths`/`detect_calls`, and no `string_targets`.

- **`erlang-gen-server`** (`rules.rs:5764-5791`), kind `Queue`, `node: false`, so root marks only. Detect: `gen_server`, `gen_statem`, `gen_event`, `gen_fsm`, `wx_object`. Methods: `handle_call`, `handle_cast`, `handle_info`, `handle_continue`, `handle_event`, `handle_sync_event`. Fixture f1: `-behaviour(gen_server)` gave "erlang-gen-server (queue, 3 entries)", namely both `handle_call` clauses and `handle_cast`.
- **`erlang-otp-lifecycle`** (`rules.rs:5794-5818`), kind `Lifecycle`. Detect: `gen_server`, `supervisor`, `supervisor_bridge`, `gen_statem`, `gen_event`, `gen_fsm`, `application`, `wx_object`. **Detect-only gap rule** (no methods; gap: `init`/`terminate`/`code_change`/`callback_mode`/`format_status` are too common to match). Fixture f1: listed under "seen, no rules" in 3 files.
- **`erlang-application`** (`rules.rs:5820-5835`), kind `Lifecycle`, `node: true`. Detect: `application`. **Detect-only gap** (`start/2`, `stop/1` not enumerated; `.app.src` `{mod, …}` not read). Fixture f1: `gamma:stop` was reported dead. `gamma:start` is live only through the generic `LANG_ENTRY` `start` name (§12).
- **`cowboy`** (`rules.rs:5837-5859`), kind `Network`, `node: true`. Detect: `cowboy`, `cowboy_req`, `cowboy_rest`, `cowboy_websocket`, `cowboy_handler`, `cowboy_router`. **Detect-only gap** (`cowboy_router:compile` dispatch tables are not decoded). Fixture f1 `web.erl` with `-behaviour(cowboy_handler)`: "seen, no rules".
- **`common-test`** and **`eunit`** are kind `Test`; see §11.
- **Rule tables not used by Erlang** (none of these match on `erlang`; verified by grep of `rules.rs`):
  - `member_sources_for`
  - `signature_methods_for`
  - `container_attributes_for`
  - `markup_kinds_for` (`rules.rs:11812`)
  - `observer_types`
  - `handler_on_enclosing_owner`
- **No multi-language rule includes Erlang.** Every `RuleSpec` has a single `language` field. `grep -n erlang rules.rs` finds only the six specs above.
- **OTP callbacks rooted outside the rule table** (dead-code only, no nodes):
  - `LANG_ENTRY` `("erlang", ["start","start_link","init"])` (`deadcode/roots.rs:26`)
  - `LIFECYCLE` `("erlang", ["handle_call","handle_cast","handle_info","terminate","code_change","handle_continue"])` (`roots.rs:96-107`)
  - Both match by simple name in **any** Erlang module, behaviour or not.
- **Visibility entries: n/a.** `-export` is not read (§12).

## 11. Test harness entries

- **`eunit`** (`rules.rs:5888-5904`), kind `Test`, `node: true`.
  - Detect: `eunit`, which matches `-include_lib("eunit/include/eunit.hrl")`. That include is recorded precisely for this purpose (`erlang.rs:72-75`; CHANGELOG.md:1925).
  - Methods: the naming patterns `*_test` and `*_test_`.
  - Fixture f1 (`alpha_tests.erl`): one node `<framework-entry>::test::eunit` with `entry` edges to `add_test` and `gen_test_`, at `low` confidence and `via: framework_entry`. `util` was not entered.
  - **The rule's `gap` text is stale.** It says cgg "has neither a suffix matcher nor a preprocessor, so no EUnit test is enumerable" (`rules.rs:5901`), but `*_test` patterns are supported and did enumerate tests in fixture f1.
  - Arity is not checked: EUnit requires 0-arity, and cgg would also enter `foo_test/1` (unverified by fixture).
  - Test `harness_reach_does_not_lift_root_coverage` (`deadcode/roots.rs:1084`) uses an eunit harness node.
- **`common-test`** (`rules.rs:5861-5886`), kind `Test`, `node: true`.
  - Detect: `ct_suite`, `common_test`, `ct_hooks`. Fixture f1: `-include_lib("common_test/include/ct.hrl")` matched.
  - Methods: `all`, `suite`, `groups`, `init_per_suite`, `end_per_suite`, `init_per_group`, `end_per_group`, `init_per_testcase`, `end_per_testcase`.
  - Fixture f1 (`alpha_SUITE.erl`): entries `all` and `init_per_suite`. **Actual testcases (`case_one/1`) are not entered**, because they are named only in the list `all/0` returns. The gap text admits this, though it still says "Enumerating only the fixed callbacks … would print a confident count … worse than declining", while the rule does exactly that now.
- **Test-file gating.** `crates/cgg-core/src/testfile.rs:110` treats `*_SUITE.erl` and `*_tests.erl` (case-insensitive) as Erlang test files. Name-only test rules apply only in a test file or in one importing the harness. Fixture f7 (no eunit include, not a test file): `plain_test/0` and `other_test_/0` were **not** entered.
- **Generic name test.** `is_test_name` falls to the default arm for Erlang, `name.starts_with("test_") || name.starts_with("Test")` (`roots.rs:975-989`). A function `test_foo` in any Erlang module is a test root. Erlang's real conventions (`_test` suffix) are not in that arm.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`: n/a.** It covers python/php/ruby only (`rules.rs:11806`).
- `--no-test-entries` turns the eunit/common-test nodes into root marks (generic behaviour; not separately verified for Erlang).

## 12. Dead-code analysis

- **Capability row** (measured, fixture f6 dead-code JSON): `class: degraded`, visibility/attributes/value_references/dispatch/exports/test_tagging all `none`, **`max_confidence: medium`**. A degraded language with no visibility or attributes cannot produce `high` findings (`signals_complete`, `deadcode/caps.rs:93-102`). Every finding carries `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references` and `language-lacks-visibility`.
- **Roots that apply:**
  - Synthesized framework entry nodes (eunit, common-test).
  - Root marks for `erlang-gen-server`.
  - `LANG_ENTRY` `main`/`start`/`start_link`/`init`, as `builtin:main` ProgramEntry.
  - `LIFECYCLE` names, as `builtin:lifecycle`.
  - `toplevel:invocation` for calls with no enclosing callable: macro bodies, error-recovered clauses (fixture f6).
  - `test_`/`Test` prefix names.
  - User `--roots`.
- **`-export([...])` is not read, so exported functions are not roots.** `facts.exports` is never filled by the plugin, and the `lang:export` rule (`roots.rs:900-925`) never fires. Fixture f1: exported `alpha:run/1`, `alpha:run/2`, `alpha:apply_it/1` and `gamma:stop/1` were all reported as `unreferenced`. Fixture f6: exported `m:go` was too. This is the largest dead-code false-positive source for library code. `-export_all`/`-compile(export_all)` is also not read.
- **Non-first clauses are reported dead.** Each clause is a node and calls bind only to the first clause. Fixture f7: `k:f@5` (the `f(N) -> N` clause of an otherwise-called `f/1`) was reported `never-referenced` at medium. The same happens for the `-else` branch of an `-ifdef` (§07). It does not happen when a rule or lifecycle name roots every clause (fixture f1 `handle_call`).
- **Function references** (`fun f/1`), MFA `spawn/apply` targets, and `-on_load(boot/0)` are invisible, so their targets are reported dead (fixture f6: `m:boot`, `m:c7`, `m:c8`, `m:c9`, `m:c13`).
- **Caps that never apply to Erlang:** `implements-external-contract`, `macro-generated` and `reflection` (no dynuse; §15). `only-used-by-tests` findings are withheld for Erlang ("1 withheld (signal not extracted)", fixture f1 report).
- **Unresolved-doubt handling.** `no-unresolved-site-with-name` appears in the evidence. A same-named unresolved site would count as doubt by the generic rules; not separately verified for Erlang.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/erlang.txt`.** 774 lines of bare identifiers, no `m:f` entries. It is loaded at `stdlib.rs:209,250` and mixes module names (`io`, `lists`, `gen_server`, `supervisor`, `erlang`, `maps`, `ets`, `ct`) with BIFs and function names (`spawn`, `apply`, `length`, `self`, `format`, `map`, `reverse`). `eunit` and `cowboy_req` are absent (checked with `grep -x`).
- **Classification.** `classify_external` (`crates/cgg-core/src/external.rs:144-200`) splits `m:f` on `:` and treats a stdlib module head as stdlib:
  - Fixture f1 with `--include-stdlib`: `<stdlib>::io::io:format`, `lists:map`, `gen_server:start_link`, `erlang:apply`, `supervisor:start_link`, plus bare `apply` and `spawn`.
  - Unknown heads (`cowboy_req:reply`, `unknown_mod:thing`) and unmatched bare names (`gen1`, `F`) become `<external>`.
- When OTP's own sources are in the tree, `lists:map` etc. bind to the in-tree definitions instead (CHANGELOG.md:1295-1297).
- **Not applicable:** `rust_types`-like wrapper tables, `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `MAYBE_TYPE_WRAPPERS`, implicit packages.
- **`--include-stdlib` / `--include-external`** mint the deduplicated exit nodes as for every language (fixture f1 output above).

## 14. Confidence & certainty

- **Paths that yield `high`** (all observed in fixtures):
  - an in-file bare call with exactly one candidate after clause narrowing (f1 `local_helper`, f7)
  - own-module `m:f` (f1 L22)
  - `m:f` into a module that exists once, or whose copies narrow to one (f2 `b:g`, `c:k`)
  - a bare call to a function from a plain `-include`d `.hrl` (f1 `hdr_fun`)
  - Elixir `:mod.f` (f9)
- **False `high`:**
  - `Mod:f()` with a variable module, bound to the local `f` (f10)
  - `m:f/2` called where only `f/1` exists, through the arity-fallback (f2 `a:t(X, X)`)
- **`medium`:**
  - bare calls bound through `-import` (always medium, and arity-blind: f2, f3)
  - `-include_lib` header functions (f11)
  - arity-mismatch fan-out across arities (f7)
- **`low`:** framework entry edges and exit-node edges.
- **Mermaid `?` marker.** An arrow whose sites are all non-high gets a leading `?`. Fixture f1: `N9 -->|?2x| N21` (two medium `bfun` sites).
- `CertainCtx`/`OwnerFacts` typed-receiver certainty does not apply: Erlang has no typed receivers. Certainty comes from the `module_certain` flag in the `m:f` path (`cross_file.rs:5597`).

## 15. Dynamic use & reflection

- **No dynuse coverage.** `crates/cgg-lang/src/plugins/dynuse.rs` `REFLECTIVE` lists no Erlang entry (`dynuse.rs:25-58`, which covers Python, Ruby, JS, Java, C# and PHP), and the plugin sets `dyn_uses: false`.
- **Not covered:**
  - Reflective forms `apply(M, F, A)`, `erlang:apply/3`, `spawn(M, F, A)`, `spawn_link/3`, `rpc:call(N, M, F, A)`, `M:F(A)` with variables, `list_to_atom` + call, `erlang:function_exported/3`. None record a literal-name use, none create an edge, and none cap findings (fixture f1/f6: `helper` via `erlang:apply(alpha, helper, …)` got no edge; `c9` via `spawn(m, c9, [])` was reported dead at medium with no reflection evidence).
  - Module loads such as `code:load_file/1` (no `module-load` screen).
- **Variable-module calls are mis-bound** rather than treated as dynamic (§03, f10).

## 16. Known gaps & limitations

- **Gaps stated in code, through `gap_for()`:**
  - `erlang-otp-lifecycle`: no entries (`rules.rs:5816`).
  - `erlang-application`: `start/2`/`stop/1` and `.app.src` (`rules.rs:5833`).
  - `cowboy`: router tables (`rules.rs:5855`).
  - `common-test`: testcases named only in `all/0` (`rules.rs:5882`).
  - `eunit`: generators and macros (`rules.rs:5901`, partly stale; §11).
- **Found while cataloguing (each with fixture evidence):**
  1. `-export` is ignored, so every exported API function with no in-tree caller is reported dead (f1, f6).
  2. `-import(M, [f/1])` drops the function list. Any bare call can bind into `M` by name, arity-blind, to the last clause's arity (f2 `h`, f3 `g/3`, f1 `bfun/2`).
  3. `Mod:f(…)` with a variable or macro module is recorded as a bare `f` and can bind the local `f` at high (f10).
  4. When the arity matches nothing, narrowing falls back to every arity. A single wrong-arity candidate becomes `high` (f2 `a:t(X,X)`).
  5. Function references `fun f/1` / `fun m:f/1` are not recorded, and there is no value-ref support (f6).
  6. Each clause is a separate callable and only the first clause gets edges, so later clauses read as dead (f7) and `-else` branches are unreachable (f8).
  7. No macro expansion: `?M(...)` calls are dropped, macro-generated functions are missing, and calls in `-define` bodies become top-level roots (f6, f8).
  8. No locals: fun-variable calls show up as `<external>::F` noise (f1, f6).
  9. No dynuse or reflection for `apply`/`spawn` MFA forms (f6).
  10. No NIF/port FFI linking (`ffi.rs`).
  11. `.app.src`, `rebar.config` and `.escript` (no shebang) are not analyzed (f8).
  12. A syntax error loses the affected clause, with no re-parse fallback (f8 broken.erl).
  13. `-include_lib` has no dedicated resolver arm. Header functions bind only at medium by name (f11).
  14. The generic `test_` prefix rule applies to Erlang, while Erlang's own `_test` suffix applies only under the eunit harness (`roots.rs:988`).
  15. Erlang → Elixir (`'Elixir.Foo':bar`) is unresolved (f9).
  16. `-callback`/`-spec`/`-on_load` are not read (f6).
- **CHANGELOG follow-ups touching Erlang:**
  - R14 remote-call fix (CHANGELOG.md:524-531).
  - The clause/arity binding that added +143k OTP edges (CHANGELOG.md:1796-1797).
  - The include_lib-for-eunit change (CHANGELOG.md:1925).
  - `-behaviour` recording (CHANGELOG.md:5046).
  - `erlang-otp` performance history (3h40m → 24.5 s, CHANGELOG.md:3846-3933). That is a C-include/import-table issue, not the Erlang plugin.
  - A U+0001 function name in OTP test data broke GraphML (CHANGELOG.md:1751).
- **Ignored tests:** none Erlang-related found. Only 3 inline tests in the plugin (`erlang.rs:291-324`).

## Summary scorecard

| section | support | one-line note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.erl`/`.hrl` + `escript` shebang; no `.escript` ext, no `.app.src`/rebar, no reparse fallback |
| 02. Callables extracted | partial | one node per function clause as `module:name`; no funs, specs, callbacks or macro-generated fns |
| 03. Call-site forms | partial | bare + `m:f` with arity; `fun f/1`, `?MACRO()`, MFA apply/spawn not recorded; `Mod:f` mis-recorded as bare |
| 04. Imports & module resolution | partial | `m:f` exact module lookup and `-include` work; `-import` function list ignored (arity-blind, over-broad); no heads.rs vocab |
| 05. Type inference & receiver typing | n/a | deliberately disabled for Erlang (`type_hints.rs:389`) |
| 06. Inheritance, dispatch & overloading | partial | clause/arity narrowing to first clause; arity-mismatch fallback; behaviours not modelled as dispatch |
| 07. Scoping & shadowing | minimal | no locals; all `-ifdef` branches merged as clauses |
| 08. Macros, preprocessor & codegen | none | no `-define` expansion; macro calls dropped; macro bodies leak top-level roots |
| 09. FFI & cross-language links | minimal | only Elixir `:mod.f` → Erlang; no NIF/port linking; Erlang → Elixir unresolved |
| 10. Frameworks & entry points | partial | gen_server root marks; otp-lifecycle, application and cowboy are detect-only gaps |
| 11. Test harness entries | partial | eunit `*_test`/`*_test_` enumerated; common-test fixed callbacks only, no testcases; stale gap text |
| 12. Dead-code analysis | minimal | degraded, max medium; `-export` not roots; non-first clauses and fun refs reported dead |
| 13. Builtins & stdlib tables | partial | 774-name `erlang.txt`, module-head classification; no `m:f` granularity |
| 14. Confidence & certainty | partial | high for unique clause/module matches; false highs on `Mod:f` and arity fallback; `-import` always medium |
| 15. Dynamic use & reflection | none | no dynuse for apply/spawn/MFA |
| 16. Known gaps & limitations | n/a | 16 gaps listed with fixture evidence |
