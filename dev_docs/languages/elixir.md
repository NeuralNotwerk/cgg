# elixir

The Elixir plugin is `ElixirPlugin`, in `crates/cgg-lang/src/plugins/elixir.rs` (1,091 lines including inline tests). It parses `.ex` and `.exs` files with the crates.io grammar `tree-sitter-elixir` 0.3.5 (`Cargo.toml:110`, `tree-sitter-elixir = "0.3"`; Cargo.lock 0.3.5). It calls the grammar through `tree_sitter_elixir::LANGUAGE` (elixir.rs:27-29). Nothing is vendored.

One recursive `ElixirWalker` (elixir.rs:53-735) does all the extraction:

- It treats every `call` node by its first child's text. `defmodule` pushes a module scope. `def`/`defp`/`defmacro`/`defmacrop`/`defdelegate`/`defguard`/`defguardp` become definitions. `alias`/`import`/`use`/`require` become imports. Anything else is a call site.
- It records `&f/n` captures as value references.
- It runs an Elixir-specific route-macro registrar pass for Phoenix and Plug.

The plugin fills only `FileFacts::definitions`, `imports` and `references`. It declares a single signal, `value_refs: true` (elixir.rs:21-26). It does not declare `visibility`, `attributes`, `exports`, `impls`, `test_defs`, `dyn_uses` or `unreachable`. It also has no `resolver_kind` override, no `ts_language_for` and no `max_line_bytes`.

Elixir-specific resolution lives in three places:

- `cgg-resolve/src/libscope.rs:1007-1144`: `build_elixir` and `resolve_elixir`, for module-qualified calls.
- `cgg-resolve/src/cross_file.rs:4163-4175`: the WS14 bare-call scope rule.
- `cgg-resolve/src/intra_file.rs:852-1010`: clause and arity handling, shared with Erlang and Haskell.

Fixtures cited below are under `<scratch>/parity/elixir/` (f1 to f7). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, and dumped with `show.py` in the same directory.

## 01. Detection & parsing

- **Extensions `.ex` and `.exs`** (elixir.rs:15-17). Fixture f1: `lib/accounts.ex` gives `detected_via: extension:.ex`; `test/accounts_test.exs` gives `extension:.exs`.
- **Shebang `elixir`** (elixir.rs:18-20). Fixture f1: an extensionless `script` starting `#!/usr/bin/env elixir` gives `detected_via: shebang:elixir` and is analyzed (`S::s` extracted).
- **Content detection: none.** `detect.rs` has no Elixir rule (grep finds no `elixir` in `crates/cgg-lang/src/detect.rs`).
- **Grammar**: one grammar for both extensions, with no dialect switch.
- **Re-parse/fallback: none.** A file with syntax errors is extracted from the error-recovered tree, which is generic behaviour. No Elixir-specific recovery exists, and none was exercised.
- **Templates not parsed.** `.heex`, `.eex`, `.leex` and `~H` sigils get no plugin or extractor: grep for `heex`/`.eex` over `crates/` finds nothing. Calls made from templates (`<%= render_x(@a) %>`, `<.component>`) are invisible.
- **`mix.exs` is only a rollup marker** (`crates/cgg/src/rollup.rs:80`). It is not read as a manifest (§04).
- **Depth skip**: shared. A tree deeper than `MAX_TREE_DEPTH` (4,000) is skipped as `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155,451`). Not exercised for Elixir.
- **Minified handling**: `max_line_bytes` is not overridden. There is no `locals` table, so the over-2,000-byte rule has nothing to drop. `--skip-minified` is generic and was not exercised (unverified).

## 02. Callables extracted

Every Elixir definition is `DefVariant::FreeFunction` (kind `function`), except the synthetic route handler, which is `NamedClosure`. The qualified name is the module scope joined with `::` plus the name: `MyApp.Accounts::get`. A nested `defmodule` pushes its own written name, so `defmodule A.B do defmodule C` gives `A.B::C::f` (elixir.rs:66-73, 213-235). `visibility` is always empty and `attributes` always empty (elixir.rs:334-347).

Supported:

- **`def` / `defp` with parens, block or one-line**: `def get(id), do: …`, `defp helper, do: :ok`, `def list do … end` (elixir.rs:239-241, 302-348). Fixture f1 extracts all of them. Test `one_line_and_guarded_defs_are_named_and_calls_carry_arity` (`def g, do: f(1)` is named `g`, not `g, do: f`).
- **Guarded heads** `defp transform(x) when is_integer(x), do: x`: the `when` binary operator's left side is the head (elixir.rs:314-318). Fixture f1 `transform` line 31. Test `calls_in_the_body_survive_head_suppression`.
- **Multi-clause functions**: each clause is its own definition under one qualified name. Fixture f1 has two `MyApp.Accounts::transform` clauses (lines 31 and 32) and two `get` clauses (get/1 and get/2). Clauses are grouped per arity at resolution time (§06).
- **`defmacro` / `defmacrop`**: a node like any function (`MyApp.Accounts::mymacro`, fixture f1).
- **`defguard` / `defguardp`**: a node (`MyApp.Accounts::is_small`, fixture f1).
- **`defdelegate`**: a node named by its head (`MyApp.Accounts::del`, fixture f1; test `unquoted_def_names_are_not_definitions`). The `to:`/`as:` target is not linked (§03).
- **Nested modules**: `defmodule Inner do def deep` inside `MyApp.Accounts` gives `MyApp.Accounts::Inner::deep` (fixture f1). Test `nested_modules_are_recorded_by_full_name`.
- **`__MODULE__` in a module name**: `defmodule __MODULE__.Sub` is expanded against the enclosing scope (`expand_module`, elixir.rs:83-91, 216).
- **Synthetic route handlers**: `get "/health" do … end` under a registrar verb with a single string argument gives `handler_at_<line>` (`NamedClosure`, attribute `synthetic`) (elixir.rs:671-711). Fixture f2 `MyApp.ApiRouter::handler_at_5`. Test `plug_router_do_block_becomes_a_named_handler`; the negative cases are `a_scope_block_is_not_a_handler` and `a_verb_without_a_route_string_captures_nothing`.
- **`signature_hint`** comes from `extract_signature` of the whole `def` text (elixir.rs:340). For a `do … end` def it carries body text, e.g. fixture f1 `list`'s hint holds its whole body. Clause arity is read from it (`clause_arity`, intra_file.rs:988+).

Not supported or only partial:

- **Unquoted names are skipped by design**: `def unquote(name)(…)` and `def unquote(:dyn)()` are dropped (elixir.rs:324-328). Test `unquoted_def_names_are_not_definitions`; fixture f3, where `:dyn` is not extracted.
- **Defs inside `quote` are mis-owned**: a literal def such as `quote do def injected, do: :i end` inside `defmacro __using__` is extracted as a function of the *macro's* module (`Base1::injected`, fixture f3). It is not tagged as generated.
- **`defprotocol` / `defimpl` are not scopes.** Their `def`s get no owner: fixture f3 has two callables both named `size` with an empty owner. Both the protocol declaration (bodiless) and the implementation become plain functions, and the protocol head has no `abstract` tag.
- **Anonymous functions** `fn x -> … end` and `&(&1 + 1)` are not callables. Calls inside them belong to the enclosing def (fixture f1, line 16 `transform(x)` inside `fn` is attributed to `list`).
- **Operator definitions are misnamed**: `def a + b, do: a` gives a callable named `a` (fixture f3 `User1::a`).
- **`defstruct`, `defexception`, `defoverridable` and `@callback`** are not callables. `defstruct`/`defexception` start with `def` and are dropped as calls by the `starts_with("def")` filter (elixir.rs:375). `@callback other_callback()` is recorded as *calls* (§12).
- **ExUnit `test "…" do … end`, `describe` and `setup` blocks are not callables** (§11).
- **Top-level `defmodule __MODULE__.Weird`** with no enclosing scope keeps the literal text `__MODULE__.Weird` (`expand_module` returns the input when the scope is empty, elixir.rs:84-90).

## 03. Call-site forms

Every non-definition `call` node is a `RefRecord` (`record_call`, elixir.rs:370-411). `arity` is always set (`call_arity`, elixir.rs:790-806).

Supported:

- **Bare call** `helper()` / `fmt(1)`: the name is the callee text and the receiver is empty. Fixture f1 `list -> helper` (high, intra-file).
- **Paren-less call with arguments** `transform 3`: a `call` node like any other. Fixture f1 line 28 `list -> transform` (high).
- **Remote call through an alias or module** `Repo.get(U, id)`, `MyApp.Accounts.list()`, `U.changeset(x)`: `remote_target` (elixir.rs:98-128) splits module and function and sets `path_receiver: true`. Test `remote_calls_name_their_module_and_aliases_are_expanded`. Fixture f1 `U.changeset(x)` gives `MyApp.Accounts.User::changeset` (high).
- **Paren-less remote zero-arity call** `Base1.base_fun`: it parses as a call with no arguments. Fixture f3 line 25 `go -> Base1::base_fun` (high).
- **`__MODULE__.f()`**: expanded to the current module. Fixture f1 line 17 `list -> MyApp.Accounts::helper` (high).
- **Erlang module call** `:ets.new(…)`, `:mymod.go(1)`: the receiver keeps its leading `:`. Fixture f1 `list -> mymod:go` (high), and `:ets.new` is stdlib.
- **Pipes**: `x |> f(a)` records `f` with arity +1 (elixir.rs:790-806). Test `one_line_and_guarded_defs_are_named_and_calls_carry_arity`. Fixture f1 `get(1) |> get(2)` binds get/1 (line 8) and get/2 (line 9). Fixture f3 `x |> Base1.base_fun()` binds.
- **Calls in guards and default arguments** are kept. Only the def head is suppressed, by start offset (elixir.rs:242-260). Tests `calls_in_the_body_survive_head_suppression` and `a_definition_head_is_not_a_call_to_itself`.
- **Recursion** is kept (test `genuine_recursion_is_still_a_call`).
- **Captures** `&f/1`, `&Mod.f/2`: a value reference with `arity` and `value_ref: true`. A remote capture sets `path_receiver` (elixir.rs:183-207, `capture_target` 755-781). These are recorded only when `ExtractCtx::reference_sites` is set (`--reference-edges` or `--dead-code`). Test `function_captures_are_value_refs_with_their_arity`. Fixture f1 with `--reference-edges`: line 14 `list -> transform` `via: reference` (high), and line 15 `list -> MyApp.Helpers::fmt` reference (high). `&(&1 + 1)` records nothing.
- **Registrar (route-macro) arguments**: see §10 (elixir.rs:413-669).

Not supported or only partial:

- **Variable-receiver calls** `x.name`, `m.s()`, `conn.assigns()`: recorded with name `x.name`, receiver `x` and `path_receiver: false` (elixir.rs:383-396). They never bind and appear as external exit nodes (`<external>::m::m.s`, fixture f3; `<external>::x::x.name`, fixture f1). There is no fan-out or typed lookup.
- **Anonymous function calls** `f.(1)`: external `<external>::f::f.` (fixture f3).
- **Special forms recorded as calls**: `case`, `with`, `cond`, `if`, `raise`, `super` and `receive` are `call` nodes, and most are missing from the stdlib list, so they become external sites. Fixture f3 has `<external>::with`, `<external>::case` and `<external>::super_thing`; fixture f1 has `<external>::raise`. See §13.
- **Module attributes recorded as calls**: `@impl true`, `@behaviour M` and `@callback f()` produce call sites named `impl`, `behaviour` and `callback`. Fixture f1's audit `unresolved_by_module` sample lists `behaviour`, `callback` and `impl`.
- **A bare zero-arity call without parens** (`helper` as an identifier) is not a `call` node and is not recorded (the walker handles `call` only, elixir.rs:203). Its fixture result is unverified.
- **`defdelegate … to: M, as: :f`**: no edge from the delegate to its target (fixture f1, `del` has no outgoing edge).
- **`apply(M, :f, args)` / `Kernel.apply`**: no edge to `f` (fixture f1, `dyn_target` stays unreferenced). See §15.
- **Callbacks passed by name**: only `&f/n` captures are value refs. An atom function name (`:f`) outside route macros is not read.

## 04. Imports & module resolution

The import records are:

- `defmodule` gives `elixir-module` (path = the full dotted module name, alias = the scope as QNs spell it) (elixir.rs:217-230).
- `alias` gives `alias` (alias name → full module) (`record_alias`, elixir.rs:130-180).
- `import`, `use` and `require` give records of that kind, with the raw argument text as the path, options included (`record_import`, elixir.rs:350-368).

Supported:

- **`alias A.B`**, **`alias A.B, as: C`** and **`alias A.{B, C}`**, including `alias __MODULE__.Sub`. Test `remote_calls_name_their_module_and_aliases_are_expanded` asserts `("Accounts","MyApp.Accounts")`, `("C","MyApp.Accounts.Cache")`, `("Repo","MyApp.Repo")` and `("Deep","MyApp.Other.Deep")`. Fixture f6 `Outer2` `Sub.s()` binds to `Outer2::Sub::s` (high).
- **Module-qualified resolution** (`LibIndex::resolve_elixir`, libscope.rs:1069-1144, called from cross_file.rs:5540-5566). The receiver resolves through the file's aliases, then a module nested in the caller's (auto-alias), then the full name. Inside that module's `(module, function)` set, the call is narrowed by arity with `first_clause_per_arity` (intra_file.rs:936-984). Test `elixir_module_qualified_calls_bind_to_the_named_module` (cross_file.rs:14245). Fixture f1 `Inner.deep()` gives `MyApp.Accounts::Inner::deep` (high).
- **External-module refusal**: a module the tree never defines gives `Scoped::External` and is counted in the external bucket. Fixture f1: `Ecto.Query.from` and `Repo.get` (Repo is in the tree but `use Ecto.Repo`). Fixture f4: `Nowhere.x()`.
- **`use` of an outside dependency**: a module that defines no `f` but `use`s an outside module is external rather than refused (libscope.rs:1131-1143). Fixture f1: `Repo.get` gives `<external>::Repo::get` because `MyApp.Repo` does `use Ecto.Repo`.
- **`not-in-imported-module`**: a module in the tree that lacks `f` gives `Refused`. It is recorded as `not-in-imported-module` only when some other callable carries the name (cross_file.rs:2810-2828). Fixture f4: `B.nope()` gives `not-in-imported-module {module: B}` when `C.nope` exists. Without another `nope` it was only counted as external (fixture f4's first run).
- **Erlang modules**: `:m.f()` binds to the Erlang `m:f` (high) (libscope.rs:1095-1108). It is refused when module `m` exists but lacks `f`, and external otherwise.
- **`import M` / `use M` for bare calls**: the module (options stripped at the first `,`) becomes an unqualified prefix (cross_file.rs:1940-1955, 2062-2075). The WS14 rule (cross_file.rs:4163-4175) lets a bare call reach only the caller's own module or an imported/used module. Test `elixir_bare_call_reaches_its_module_and_imports` (cross_file.rs:14125). Fixture f6: `Z.z` calls `zz()`, which exists only in un-imported `W`, and is refused as `candidates-in-other-files`.

Not supported or only partial:

- **`import M, only: […]` / `except: […]` filters are ignored.** Only the module is kept (cross_file.rs:2066-2071).
- **Imports are file-scoped, not module-scoped.** Fixture f5: module `E`, in the same file as `A` (which does `import B`) but not importing anything, binds `helper()` to `B::helper` at **high**.
- **`require M`** is recorded but is no resolution input. Its effect on macro calls is not modelled (no code reads kind `require` for Elixir; unverified beyond grep).
- **No manifest reading**: `mix.exs` deps and `apps/*` umbrella paths are not read, and `FileFacts::packages` is never set by the Elixir plugin.
- **`heads.rs` / `modules.rs` do not cover Elixir.** Grep finds no `elixir` in either; the external refusal lives in `libscope.rs` instead.
- **Erlang → Elixir is not linked**: `'Elixir.A':a()` from an `.erl` file is external (fixture f5).
- **Functions injected by an in-tree `use` macro are not modelled.** CHANGELOG.md:1539-1542 says these are refused. In fixture f3, `injected()` bound to the template def `Base1::injected` only because of the mis-owned quote def and `import Base1` (§02).

## 05. Type inference & receiver typing

Not applicable. The Elixir plugin records no `local_types`, parameter types, `scoped_types`, `typed_receiver`, `weak_receiver_type`, `arg_type` or `implicit_receiver`. Its only `RefRecord` fields are name, receiver, arity, `path_receiver`, `value_ref`, context and route (elixir.rs:400-408, 610-634).

- `type_hints.rs:347` lists `elixir` only in `lang_tallies_table!`, which holds per-language perf counters (type_hints.rs:300-333). No type rule is involved.
- Struct literals do not type their variables: `x = %U{}; x.name` gives the external `x.name` (fixture f1).
- Typespecs (`@spec`) are not read; they parse as module-attribute calls.
- `declared_type`, wrappers, `CAST_INTRINSICS`, `CONSTRUCTING_METHODS` and closure parameter typing: none apply.

## 06. Inheritance, dispatch & overloading

- **Clause language**: `is_clause_language` holds `erlang | elixir | haskell` (intra_file.rs:852-860). Same-name candidates are one function per arity, not overloads to guess between.
- **Intra-file arity narrowing**: `clause_targets` (intra_file.rs:886-934) picks the first clause per arity. It is High when exactly one function remains. Fixture f1: `get(1) |> get(2)` binds get/1 and get/2 separately (high each). `transform(x)` binds the first `transform` clause (line 31) only.
- **Cross-file module-qualified arity narrowing**: `first_clause_per_arity` (intra_file.rs:936-984), applied in cross_file.rs:5547-5552. Fixture f6: `W.m(1)` binds `m/1`.
- **Default arguments** are not expanded into extra arities. `clause_arity` counts commas (intra_file.rs:988+), so `def m(a, b \\ 1)` is m/2 only. In fixture f6, `W.m(1)` bound only to `m/1`.
- **Gap: arity is ignored for imported bare calls.** Step 1b (cross_file.rs:5943+) looks up `prefix.f` without arity:
  - Fixture f5: `h2(1)` and `h2(1, 2)` under `import B` both bind to `B::h2/2` (line 4) at medium, and mermaid shows `?2x`.
  - Fixture f1: `fmt(1)` under `import MyApp.Helpers, only: [fmt: 1]` binds `fmt/2` (helpers.ex:3), so the wrong clause is chosen.
- **`OVERLOAD_ARITY_LANGS`** is `["java","scala","cpp"]` (type_hints.rs:1583). Elixir is not in it, and uses the clause path instead.
- **`INHERITANCE_LANGS`** does not include Elixir (dispatch.rs:110+), so `inheritance_fanout` does not run for it. `trait_impl_target` is never set, so `--dynamic-dispatch` adds nothing.
- **Behaviours** (`@behaviour M`, `@callback`, `@impl true`) are not read as contracts, and are recorded as call sites instead (§03). OTP callbacks are rooted by name rules instead (§10, §12).
- **Protocols** (`defprotocol` / `defimpl … for:`): no dispatch link, and `MyProto.size(x)` is external (fixture f3), because the protocol is not an `elixir-module`.
- **`super`** in a `defoverridable` override is an external site `super_thing`-style (fixture f3). There is no base chain.
- **`use`** is recorded per module (`ex_uses`, libscope.rs:1035-1065). It is used only for the external-vs-refused decision and as a bare-call prefix. No mixin members are injected.
- **`member_of_calling_class`, `overrides_below`, `super` resolution and extension methods**: not applicable.

## 07. Scoping & shadowing

- **No `FileFacts::locals` table**: `locals::spec` has no `elixir` arm (locals.rs:380-393). So `shadowed-by-local` never fires. Elixir variables cannot be called bare (`f.()` is required), so this costs little.
- **Module-scoped bare calls cross-file**: WS14 (§04).
- **Gap: same-file modules defining the same name are ambiguous.** Fixture f6 has `X.h` and `Y.h` in one file. `X.go`'s `h()` and `Y.go2`'s `h()` are both `ambiguous-in-file` instead of binding to the caller's own module. Intra-file does not prefer the enclosing module for Elixir.
- **Nested module visibility**: `X.In.i` calling `h()` (defined in the parent `X`) is not bound. That is correct Elixir, because nested modules do not see the parent's functions. Fixture f6 reports it `ambiguous-in-file`.
- **Lexical narrowing of nested definitions and positional shadowing**: not applicable. Elixir is not in either list (CLAUDE.md architecture), and there are no nested named functions.
- **Conditional definitions** (`if Code.ensure_loaded?(X) do def … end`): no `DefRecord::conditional` is set. A def inside an `if` is extracted like any def (unverified by fixture).

## 08. Macros, preprocessor & codegen

- **`quote` / `unquote`**:
  - A def whose name is `unquote(...)` or `unquote_splicing(...)` is dropped (elixir.rs:324-328; CHANGELOG.md:1052-1058, A11). Test `unquoted_def_names_are_not_definitions`.
  - A literal-named def inside `quote` is extracted under the macro's module (fixture f3 `Base1::injected`). It is not tagged `macro:<name>`.
  - `quote` and `unquote` are stdlib names (stdlib/elixir.txt). Fixture f1 `--include-stdlib`: `mymacro -> <stdlib>::quote`.
- **`defmacro` bodies** are ordinary callables, and their calls are ordinary sites.
- **Macro calls at module level** (`MyApp.Gen.defgetter(:x)`, `plug :match`, `schema "users" do`) bind but draw no edge, because no callable encloses them (CHANGELOG.md:1539-1541). They are recorded as `no-enclosing-callable`, which feeds the by-name `toplevel:invocation` root (§12).
- **`use`-generated code** (`__using__`): not expanded (CHANGELOG.md:1541-1542).
- **There is no `macro:<name>` tagging and no `macro-generated` dead-code cap for Elixir.** `rust_macro.rs`/`c_macro.rs` are Rust/C only.
- **`name_mentions`**: not populated by the Elixir plugin.
- **Preprocessor, `#if` and templates**: not applicable.

## 09. FFI & cross-language links

- **Elixir → Erlang**: `:mod.f()` binds to the in-tree Erlang `mod:f` at high (libscope.rs:1095-1108). Fixture f1: `list -> mymod:go` (high). This is a resolver path, not `ffi::link_ffi`.
- **Erlang → Elixir** (`'Elixir.A':a()`): not linked, and external (fixture f5).
- **`ffi::link_ffi`** has no Elixir coverage. There is no Rustler `#[rustler::nif]`, no `:erlang.load_nif` and no Port or NIF stubs: grep for `rustler`/`nif` in `ffi.rs` finds nothing.
- **Descriptor links**: `descriptor::link_descriptors` is language-agnostic. It needs the candidate's lowercased simple name to equal the rpc's (descriptor.rs:79-140). Elixir gRPC handlers are snake_case (`say_hello` against `SayHello`), so they cannot match. Fixture f7: even `def ping` under `Hw.Greeter.Server` against `rpc Ping` drew no edge, while a Python `GreeterServicer.Ping` in the same fixture did. The cause of the `ping` miss is unverified.
- **Markup code-behind**: not applicable (`markup.rs` is C#-only).

## 10. Frameworks & entry points

Every rule whose `language` is `elixir` (rules.rs:2946-3123; 9 rules). No multi-language rule names Elixir. Detection matches each rule's `detect` prefixes against the file's import records (`use`/`import`/`alias`/`require`), as in `frameworks/mod.rs:258-276`. Benchmark coverage: `scripts/benchmark.sh:84,174,216,426`.

| rule id | kind | node | trigger | evidence |
| --- | --- | --- | --- | --- |
| `otp` | lifecycle | `false` (root mark only) | detect `GenServer`, `Supervisor`, `DynamicSupervisor`, `Agent`; methods `init handle_call handle_cast handle_info handle_continue terminate code_change format_status child_spec` | rules.rs:2946-2972; fixture f1: `init`/`handle_call`/`handle_cast` rooted, detail `otp:base-type` |
| `otp-application` | lifecycle | `false` | detect `Application`; methods `start prep_stop config_change start_phase` | rules.rs:2973-2989; fixture f2: `MyApp.App::start` rooted |
| `mix-task` | cli | `true` | detect `Mix.Task`; method `run` | rules.rs:2990-3006; fixture f2: `<framework-entry>::cli::mix-task::Mix.Tasks.Hello::run` |
| `phoenix-channel` | network | `true` | detect `Phoenix.Channel`; methods `join handle_in handle_out` | rules.rs:3007-3023; fixture f2: `join`, `handle_in` |
| `phoenix-socket` | network | `true` | detect `Phoenix.Socket`; method `connect` | rules.rs:3024-3040; fixture f2: `connect` (but `id/1` is not, and is flagged dead) |
| `phoenix-liveview` | network | `true` | detect `Phoenix.LiveView`, `Phoenix.LiveComponent`; methods `mount handle_event handle_params handle_async` | rules.rs:3041-3057; fixture f2: `mount`, `handle_event`; `render/1` is **not** listed and is flagged dead |
| `plug` | network | `true` | detect `Plug.Conn`, `Plug.Builder`; method `call` | rules.rs:3058-3074; fixture f2: `MyApp.AuthPlug::call` (`init` is rooted by `LANG_ENTRY` instead) |
| `phoenix` | network | `true` | detect `Phoenix.Router`, `Phoenix.Controller`, `Phoenix.VerifiedRoutes`; registrars `get post put patch delete options head live resources forward` | rules.rs:3075-3106; fixture f2: `get("/")` → `PageController::home`, `post("/x")` → `create` |
| `plug-router` | network | `true` | detect `Plug.Router`; registrars `get post put patch delete options match` | rules.rs:3107-3123; fixture f2: `get("/health")` → `handler_at_5` |

How the registrar pass reads route macros (elixir.rs:413-669):

- **Gate**: the verb must be a registrar verb of the Elixir rules (`registrar_verbs_for`, rules.rs; `ExtractCtx::is_registrar_verb`, lib.rs:202+), and the *first* argument must be a string literal (elixir.rs:427-450). Test `a_verb_without_a_route_string_captures_nothing`, where `Map.get` and `Map.put(…, "https://…")` produce nothing.
- **Argument shapes**:
  - an `alias` gives the module's last segment as a value ref;
  - an `atom` gives `Owner::action` (string-ref) plus the bare `action` (value-ref);
  - an `identifier` gives a value-ref;
  - a `dot` (`&Mod.handler/2`) gives `Mod::handler` plus `handler`;
  - `keywords`/`pair`/`list`/`tuple`/`unary_operator`/`binary_operator` are recursed into, depth ≤ 3;
  - a nested `call` contributes its callee only.

  Tests: `phoenix_route_macro_names_its_controller_and_action` and `keyword_and_captured_targets_are_read`.
- **Inline handler**: `get "/x" do … end` with the path as the only argument gives `handler_at_<line>` (§02).
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for`, `self_module_markers_for`, `detect_calls_for`**: none has an Elixir arm (rules.rs:11666-11880 match only other ids). The `gap` string is empty for all 9 rules.

Not supported or only partial:

- **`resources "/users", UserController`**: no action atoms are written, so `index`/`show`/… get no entry. Fixture f2: `UserController::index`/`show` are reported dead.
- **`live "/dash", DashLive`** and **`forward "/api", MyApp.ApiRouter`** produce only a module value-ref, which has no callable. Fixture f2: unresolved `value-ref-no-enclosing` for `DashLive`/`ApiRouter`. LiveView callbacks are covered by the method rule.
- **`match _ do … end`** (Plug.Router catch-all) has no string first and gets no handler (fixture f2).
- **Function plugs** `plug :authenticate` and `pipe_through` are not registrar verbs, so module plug functions are not entered.
- **Duplicate rules on one route**: in fixture f2, every route was minted under *both* `phoenix` and `plug-router` (e.g. `phoenix::get("/")` and `plug-router::get("/")` → `home`). The tree used both frameworks in different files; the exact detection scoping is unverified.
- **No rules for** Oban (`perform/1`), Ecto (`changeset`, `Ecto.Schema`), Absinthe, Broadway, Nerves, Phoenix components (`attr`/`slot`, function components) or `Phoenix.Controller` actions not named by a route. Fixture f2: `MyApp.Worker::perform` and `MyApp.Schema::changeset` are flagged dead, and the framework coverage reports them as neither recognised nor "seen, no rules".

## 11. Test harness entries

- **No Elixir test rule.** None of the 53 `TrustKind::Test` rules in rules.rs is `language: "elixir"` (grep `-B3 TrustKind::Test`). So ExUnit gets no `<framework-entry>::test::…` node and `--no-test-entries` has nothing to change. The dead-code capability row reports `test_tagging: none` (fixture f1 dc.json).
- **Test-file gating only**: `testfile.rs:109` treats `*_test.exs` as a test file, which drives `--include-tests` scoping.
- **ExUnit blocks are not callables** (`test "…" do … end`, `describe`, `setup`), so:
  - a bare call inside one (`tested_helper()`) is a `no-enclosing-callable` site. It roots *every* Elixir callable of that simple name through `toplevel:invocation` (§12). Fixture f1: `MyApp.AccountsTest::tested_helper`, detail "called from module top level at ./test/accounts_test.exs:7".
  - a module-qualified call inside one (`MyApp.Accounts.get(1)`) is recorded as `no-candidate-in-file` and makes nothing live. Fixture f1: `--why-live 'MyApp.Accounts::get$'` gives "NOT REACHED".
- `TEST_CLASS_MEMBERS_ARE_ROOTS` is `["python","php","ruby"]` (rules.rs:11806). Elixir is not in it.
- `doctest MyMod` is not read.

## 12. Dead-code analysis

Capability row for Elixir, from fixture f1 `--dead-code-format json`:

- `class: degraded`, `visibility: none`, `attributes: none`, `value_references: full`, `dispatch: none`, `exports: none`, `test_tagging: none`.
- `max_confidence: medium`, so **no Elixir finding can be high**.
- Every finding carries the risks `language-lacks-attributes`, `language-lacks-dispatch-model` and `language-lacks-visibility`.

Roots:

- **`LANG_ENTRY`**: `start`, `start_link`, `init` by name in any Elixir module (deadcode/roots.rs:25), plus the universal `main`. Fixture f1: `start_link` (`builtin:main`). Fixture f2: `MyApp.AuthPlug::init`.
- **`LIFECYCLE`**: `handle_call handle_cast handle_info terminate code_change handle_continue` (roots.rs:108-117), by name.
- **Framework rules** (§10): `framework-callback` roots.
- **`toplevel:invocation`** (roots.rs:555-613): any `no-enclosing-callable` / `value-ref-no-enclosing` site roots every callable of the same language and simple name. That includes:
  - module-body macro calls;
  - route arguments (fixture f2: `home`/`create` rooted from router.ex:4-5);
  - ExUnit bodies;
  - **`@callback other_callback()`** declarations (fixture f1: `MyApp.Server::other_callback`, "called from module top level at ./lib/server.ex:13"). This root comes from a by-name accident, not from behaviour knowledge.
- **Root reachability**: `enabled:min-coverage-25pct`. Fixture f1: 22% reachable, so 10 reachable-only-from-dead-code findings were withheld ("root coverage too low").

Signals and caps:

- **`def` vs `defp` is not recorded** (visibility is always empty, elixir.rs:345). A public library `def` with no in-tree caller is reported like a private one (fixture f1 `unused_public` and `unused_private` both medium).
- **No `implements-external-contract`, `macro-generated` or `reflection` cap for Elixir.** `caps.rs` has no Elixir entry, and `dynuse` does not cover it (§15).
- **No unreachable-code signal**: `cfg::unreachable_after_terminator` is not wired. Fixture f1: `unused_after_raise -> unused_private` after `raise` is a normal edge.
- **False positive: later clauses of a called function are flagged.** Only the first clause per arity receives the edge (§06). Fixture f1: `MyApp.Accounts::transform` line 32 is reported unreferenced although `transform/1` is called (its line-31 clause got the edges).
- **Unresolved-doubt**: findings list `no-unresolved-site-with-name` when no same-name site exists. Behaviour beyond that is generic.

## 13. Builtins & stdlib tables

- **`stdlib/elixir.txt`**: 255 lines (crates/cgg-core/src/stdlib/elixir.txt), loaded at stdlib.rs:208,249.
  - It mixes module names (`Enum`, `Map`, `String`, `GenServer`, `Logger`, `Kernel`, `Process`, `Task`, `ExUnit`), Kernel functions (`apply`, `send`, `spawn`, `self`, `is_integer`, `map_size`, `to_string`, `inspect`, `quote`, `unquote`), ExUnit macros (`assert`, `test`, `describe`) and Erlang atoms (`:code`, `:crypto`, `:ets`, `:telemetry`, `:timer`).
  - Fixture f1 `--include-stdlib`: `<stdlib>::Enum::map`, `<stdlib>:::ets::new`, `<stdlib>::String::upcase`, `<stdlib>::Logger::info`, `<stdlib>::apply`, `<stdlib>::is_integer`, `<stdlib>::GenServer::start_link`.
- **Missing from the table, so classed external**: `case`, `with`, `cond`, `if`, `unless`, `fn`, `receive`, `raise`, `throw`, `super`, `defstruct`, `__MODULE__` and `:erlang` (grep `-cx` = 0 each). Fixture f1: `<external>::raise`. Fixture f3: `<external>::with`, `<external>::case`.
- **No `rust_types`-like, `CAST_INTRINSICS`, `CONSTRUCTING_METHODS` or implicit-package tables** for Elixir.
- **`--include-stdlib` / `--include-external`** mint the usual exit nodes (fixture f1 out2.json).

## 14. Confidence & certainty

Paths that yield **high**:

- An intra-file call that leaves exactly one clause-function (`clause_targets`, intra_file.rs:927-931). Fixture f1: helper, transform, get/1, get/2.
- A module-qualified call through `resolve_elixir` whose arity narrowing leaves one function (`module_certain = found.len() == 1`, cross_file.rs:5547-5552). Fixture f1: `U.changeset`, `__MODULE__.helper`, `:mymod.go`. Fixture f4: `B.b`.
- A bare call through an unqualified import prefix when no other opened module declares the name (cross_file.rs:5945-5975). Fixture f5: `helper()` gives `B::helper` (high, including the wrong-module `E::e` case).
- Route value refs bind as reference-edges. Framework-entry edges are `low` (fixture f2).

What forces **medium**:

- Several arities remaining. Imported bare calls into a multi-arity function give medium, and can be the wrong clause (fixture f5 `h2`, fixture f1 `fmt`).
- **Mermaid `?` marker**: fixture f5 mermaid shows `N0 -->|?2x| N4` for the two medium `h2` edges.
- `--guess-unique-names` (`low`) is Rust-only and not applicable.

## 15. Dynamic use & reflection

- **No dynuse coverage.** `REFLECTIVE` in `plugins/dynuse.rs:26-55` lists Python, Ruby, Java, C#, JS, TS, PHP and Go only. `DYN_MODULE_LOADERS` (facts.rs:165) is `require`, `importScripts`, `import_module`.
- Fixture f1: `apply(__MODULE__, :dyn_target, [])` creates no edge and no reflection cap. `dyn_target` is reported unreferenced at medium.
- Not modelled: `apply/2,3`, `Kernel.apply`, `function_exported?`, `Code.eval_string`, `Code.ensure_loaded`, `Module.concat`, `String.to_existing_atom`, `GenServer.call(pid, msg)` message dispatch to `handle_call` (rooted by name instead), `send`/`receive`, and dynamic `:"Elixir.#{mod}"`.

## 16. Known gaps & limitations

The gaps below are ordered by how much they change the graph.

1. **Imported bare calls ignore arity.** `import B` + `h2(1)` binds `h2/2` at medium (fixtures f5 and f1 `fmt`; cross_file.rs:5943+ step 1b has no clause narrowing).
2. **Imports are file-scoped.** Another module in the same file inherits an `import` and binds at high (fixture f5 `E::e -> B::helper`).
3. **Same-file modules sharing a function name are `ambiguous-in-file`.** The caller's own module is not preferred (fixture f6).
4. **No visibility (`def` vs `defp`), attributes (`@impl`, `@doc false`), behaviours or protocols.** Dead code caps at medium for the whole language (fixture f1 capabilities).
5. **Later clauses of a called function are flagged dead** (fixture f1 `transform` line 32).
6. **ExUnit has no harness rule and test blocks are not callables.** Calls from tests either root by bare name across the tree or are lost (fixture f1 `--why-live`).
7. **`toplevel:invocation` roots by simple name across the language.** `@callback f()`, route atoms and test bodies root every same-named Elixir function (roots.rs:555-613; fixture f1 `other_callback`).
8. **`defprotocol`/`defimpl` have no owner, and protocol calls are external** (fixture f3).
9. **`use`/`__using__`-injected functions are not modelled.** Quote-template defs are mis-owned to the macro's module (fixture f3; CHANGELOG.md:1539-1542).
10. **Special forms and module attributes are recorded as calls** (`case`, `with`, `raise`, `@impl`, `@callback`). They add external/unresolved noise because the stdlib list lacks them (§13).
11. **Variable-receiver calls never bind** (`conn.assigns()`, `m.s()`, which go external). There is no type inference at all (§05).
12. **No reflection handling** (`apply/3`) (§15).
13. **Phoenix gaps**: `resources`, `live`/`forward` targets, function plugs, `render/1` and `Socket.id/1`. There are no Oban/Ecto/Absinthe rules, and `phoenix` and `plug-router` double-mint the same route (fixture f2).
14. **Erlang → Elixir calls are not linked**, Rustler NIFs are not modelled, and gRPC descriptor links never reach Elixir handlers (fixtures f5 and f7).
15. **Templates (`.heex`/`.eex`, `~H`) are not parsed.**
16. **Default arguments (`\\`) are not expanded into extra arities**, and operator definitions are misnamed (`def a + b` gives `a`).
17. **No unreachable-after-`raise` signal and no `locals` table.**
18. No `#[ignore]`d Elixir tests were found (grep of `crates/*/tests` finds only `detect_prefixes.rs:45`, which builds a `use` import form). Elixir rule coverage in Rust tests is limited to the plugin's 13 inline tests and the 2 cross_file tests.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | full | `.ex`/`.exs` + `elixir` shebang; no templates (`.heex`/`.eex`), no content sniff |
| 02 Callables extracted | partial | all `def*` forms, clauses, nested modules, route handlers; no protocol/impl owners, quote defs mis-owned, operator defs misnamed |
| 03 Call-site forms | partial | bare, paren-less, remote, `__MODULE__`, `:erl`, pipes (+1 arity), captures; variable receivers, `f.()`, special forms and `@attr` become external noise |
| 04 Imports & module resolution | partial | alias (all forms) + module-qualified resolution solid; `only:`/`except:` ignored, imports file-scoped, no mix.exs |
| 05 Type inference | none | no local/param/struct typing; variable-receiver calls never bind |
| 06 Inheritance/dispatch/overloading | partial | clause-per-arity narrowing intra-file and module-qualified; imported bare calls ignore arity; no behaviours/protocols/`super` |
| 07 Scoping & shadowing | minimal | WS14 module-scoped bare calls; no locals; same-file modules sharing a name are ambiguous |
| 08 Macros & codegen | minimal | `unquote` names skipped; no `use`/`__using__` expansion, no `macro:` tagging |
| 09 FFI & cross-language | minimal | Elixir→Erlang `:m.f` high; Erlang→Elixir, NIFs and gRPC descriptors not linked |
| 10 Frameworks & entry points | partial | 9 rules (OTP, Application, Mix, Phoenix router/channel/socket/LiveView, Plug, Plug.Router); missing resources/live/forward/plugs/Oban/Ecto |
| 11 Test harness entries | none | no ExUnit rule; only `_test.exs` file gating; test blocks are not callables |
| 12 Dead-code analysis | partial | name-based OTP and framework roots; no visibility, so max confidence is medium; later-clause false positives; by-name toplevel roots |
| 13 Builtins & stdlib | partial | 255-line list; misses `case`/`with`/`if`/`raise`/`receive`/`super`/`:erlang` |
| 14 Confidence & certainty | partial | high for single clause-function or module-qualified match; medium on multi-arity; the high import path can be wrong-module |
| 15 Dynamic use & reflection | none | no `apply/3` or `Code.*` handling, no reflection cap |
| 16 Known gaps | n/a | 18 gaps listed above |
