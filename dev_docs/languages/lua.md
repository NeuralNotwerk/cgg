# lua

The Lua plugin (`crates/cgg-lang/src/plugins/lua.rs`, 401 lines, `LuaPlugin`, id `"lua"`) is a small hand-written tree-sitter walker over the `tree-sitter-lua` crate (`Cargo.toml:98`, `tree-sitter-lua = "0.5"`, bound as `tree_sitter_lua::LANGUAGE`, lua.rs:21-23; nothing is vendored). It extracts named functions (global, `local`, table-field, colon-method, assigned and named-table-literal functions), records every `function_call` as either a bare or a receiver-split call, and turns `require(...)` into an `ImportRecord { kind: "require" }` with the bound local name as alias (lua.rs:286-312). There is no per-plugin resolver kind: `resolver_kind` no longer exists in `cgg-lang` (grep finds no occurrence), so every Lua file goes through the shared resolver. Resolution is the generic intra-file / cross-file pipeline plus Lua-specific pieces: lexical narrowing and positional shadowing in `intra_file.rs`, the `ModuleIndex`/`FileModules` require resolver in `cgg-resolve/src/modules.rs` (`resolve_lua`, modules.rs:1650-1708), and a parameter/local table in `cgg-lang/src/locals.rs` (`LUA_SPEC`, locals.rs:206-219). It reads no attributes, no visibility, no classes/bases, no types, no value references, no registrar arguments and no dynamic-use literals; `ExtractCtx` is ignored (`_ctx`, lua.rs:27).

Fixtures used for this document live in `<scratch>/parity/lua/` (`p1/` general shapes + module resolution + dead code, `p2/` frameworks + table/assignment shapes + skips, `p3/` shadowing + shebang), run with `cgg 0.11.0` (`target/release/cgg`).

## 01. Detection & parsing

- **Extension** `.lua` only (lua.rs:15-17). Fixture: `p1/lib/util.lua` → `detected_via: extension:.lua`.
- **Shebangs** `lua`, `luajit` (lua.rs:18-20). Shebang matching is substring-based (`detect.rs:63-70`), so `#!/usr/bin/env luajit` matches the first needle `lua`. Fixture `p3/script` (no extension, `#!/usr/bin/env luajit`) → `detected_via: shebang:lua`; `p1/main.lua` with `#!/usr/bin/env lua` → `shebang:lua` (shebang beats extension, same rule as `detect.rs::python_shebang_beats_extension`).
- **Content detection**: none. No header heuristic for Lua in `crates/cgg-lang/src/detect.rs` (grep for `lua` there finds nothing).
- **Not detected**: `.luau` (Roblox Luau), `.nse` (Nmap scripts), `.rockspec`, `.p8` (PICO-8), `.wlua`, `.luacheckrc`/`.busted` config files, Lua embedded in other files (nginx `*_by_lua_block`, Neovim `vim.cmd` heredocs, Redis `EVAL` strings). Evidence: `extensions()` lists `.lua` only (lua.rs:16).
- **Grammar**: single grammar, `tree_sitter_lua::LANGUAGE` (lua.rs:22). No per-file grammar choice (`ts_language_for` not overridden).
- **Re-parse / fallback**: none. A file with parse errors is still extracted; `parse_status` is `"error"` (`crates/cgg/src/lib.rs:457-461`). No `#if`-style re-parse as C# has.
- **Depth skip**: a tree deeper than `MAX_TREE_DEPTH` (4000) is skipped as `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:451-456`). Fixture `p2/deep.lua` (5000 nested parens) → `file_skipped ... {'kind': 'too-deep', 'detail': 4000}`.
- **Minified files**: any line over 2,000 bytes (`MINIFIED_LINE_BYTES`, `crates/cgg/src/lib.rs:159`) disables `locals::collect` for the file (lib.rs:480-487); callables and calls are still extracted. Fixture `p2/min.lua` (one 3.2 KB line) → analysed, `parse_status ok`.

## 02. Callables extracted

All definitions go through `record_def` (bare name, `DefVariant::FreeFunction`, lua.rs:232-251) or `push_def` (table member, `DefVariant::InherentMethod`, lua.rs:207-230). `signature_hint` is `extract_signature` of the defining node's text. `visibility` is always empty and `attributes` always empty (lua.rs:226-227, 247-248).

Supported:

- **Global function declaration** `function f() end` → simple `f`, qn `f`, kind `function` (lua.rs:64-69 → `record_named_def` identifier arm, lua.rs:150-155). Fixture p1: `run2`, `helper_str`, `unused_global`.
- **Local function** `local function f() end` → simple `f`, qn `f`, `FreeFunction` (lua.rs:111-121). Fixture p1: `outer`, `unused_local`; test `extracts_definitions` (lua.rs:388).
- **Table-field function** `function M.f() end` → simple `f`, qn `M.f`, `InherentMethod` / kind `method` (lua.rs:156-164). Fixture p1 `M.decode`, `M.helper`; unit test `table_functions_are_named_by_their_field_and_calls_split_their_receiver` (lua.rs:346) asserts `("f","M.f")`.
- **Nested table path** `function M.sub.deep() end` → simple `deep`, qn `M.sub.deep` (fixture p1). `function obj.a.b:c() end` → simple `c`, qn `obj.a.b:c` (fixture p2).
- **Colon method** `function M:encode(x) end` → simple `encode`, qn `M:encode` (the `:` is kept in the qn; `method_index_expression` arm, lua.rs:156-160). Unit test asserts `("h","M:h")`.
- **Assigned function** `M.a = function() end`, `local f = function() end`, `f = function() end` → named by the assignment target (lua.rs:71-93). Target an identifier → `FreeFunction` (fixture p1: `cb` from `local cb = function() ... end`); target a dot/method index → `InherentMethod` (fixture p1 `M.assigned`; unit test `("a","M.a")`).
- **Multiple assignment** `local x, y = function() end, function() end` → both `x` and `y` (targets and values zipped, lua.rs:85-89). Fixture p2 nodes `x`, `y`.
- **Function in a named table literal** `local t = { k = function() end }` and `M.cfg = { k = function() end }` → simple `k`, qn `t.k` / `M.cfg.k` (lua.rs:94-110, `table_binding` lua.rs:172-187). Fixture p1 `handlers.greet`; unit test `("k","t.k")`.
- **Assigned function inside a function body** `t.m = function() end` → `t.m` (fixture p2).
- **Nested functions** `local function outer() local function inner() end end` → both are callables; the inner one is `inner` with **no owner prefix**: `self.scope` is never pushed (lua.rs:37, 54-60), so every bare-name qn is the simple name regardless of nesting. Fixture p1 (`outer`, `inner`), p2 (`p`/`h` and `q`/`h`, two callables with identical qn `h`).
- **Varargs** `local function n(...) end` → callable (fixture p2 `n`); varargs are not otherwise modelled.

Not supported / partial:

- **Anonymous functions** — `function() ... end` passed as an argument, returned, or stored anywhere not covered above, is not a callable; calls inside it belong to the nearest enclosing named callable or, at file top level, to none (`no-enclosing-callable`). Fixture p1 `spec/util_spec.lua`: `describe(..., function() it(..., function() ... end) end)` produces no callables and three `src: None` sites.
- **Module returning a table literal** `return { f = function() end }` — the table is not bound to a name, so `table_binding` returns `None` (lua.rs:173-177) and nothing is recorded. Fixture `p2/ret.lua` → zero callables; a caller's `r.f()` lands in the external bucket (`no-candidate-in-file`).
- **Functions in a nested table literal** `{ ["/path"] = { GET = function() end } }` — the inner table's parent is a `field`, not an assignment, and bracket keys are not `identifier`s (lua.rs:100), so nothing is recorded. Fixture p1: no `GET` node.
- **Bracket-key fields** `t["name"] = function() end` and `t[k] = function` — the target kind is `bracket_index_expression`, which `record_named_def` ignores (lua.rs:166 `_ => {}`) (inferred from code; not run separately).
- **Constructors / classes** — no class concept; `Animal.new` is an ordinary `InherentMethod`. No `CallableKind::Constructor`, no `ClassDecl`. `setmetatable`-based classes are invisible as types.
- **Metamethods** (`__index`, `__call`, `__add`, …) are ordinary members if written as named functions; no operator modelling.
- **Overloads / abstract members / generated code**: not applicable to Lua (no overloading, no interfaces, no macros).
- **Owner/qualified names**: the qn is the target text with whitespace removed (lua.rs:162, 186). Since the qualified name keeps `:`, `M.f` and `M:f` are different qns for the same table slot.

## 03. Call-site forms

Every `function_call` node is handled by `record_call` (lua.rs:253-322). No `arity`, `value_ref`, `paren_less`, `typed_receiver`, `path_receiver`, `dot_call` or `weak_receiver_type` flags are ever set (lua.rs:270-276, 315-321 use `..Default::default()`).

Supported:

- **Bare call** `helper()` → `RefRecord { name: "helper", receiver_hint: "" }` (lua.rs:315-321). Unit test asserts `("helper","")`. Fixture p1 `run → helper_str`.
- **Dot call** `M.f()`, `a.b.f()` → name = field, receiver_hint = table text (`M`, `a.b`) (lua.rs:259-279). Unit test `("g","M")`, `("x","util")`.
- **Colon call** `obj:m()`, `self:f()` → name = method, receiver_hint = table text. Unit test `("f","self")`. The implicit `self` argument is not distinguished from a dot call.
- **String-argument call** `f"abc"` and **table-argument call** `f{1,2}` → ordinary calls (grammar `function_call`). Fixture p1 `run → helper_str`, `run → helper_tbl` (both high).
- **Chained call** `Animal.new("y"):speak()` → recorded with name `speak` and receiver_hint `Animal.new("y")` (the full call text). Fixture p1 line 57: inner `Animal.new` bound high; outer `speak` unresolved `candidates-in-other-files` / external bucket. Return types are never used.
- **require as import** `require("m")` / `require "m"` / `require 'm'` → not a call; an `ImportRecord` (lua.rs:286-312). See §04.
- **Inline require call** `require("ret").g()` → the import is recorded with empty alias; `g` is recorded with receiver_hint `require("ret")` and is not resolved through the module (fixture p2 line 4: external bucket `no-candidate-in-file`).

Not supported / partial:

- **Value references / callbacks** — `pcall(run2)`, `table.sort(t, cmp)`, `local f = outer` are not recorded as references at all (no `value_ref`, and `ExtractCtx::reference_sites` is ignored). Fixture p1: `run2` reported dead though passed to `pcall`; `--reference-edges` adds nothing for Lua.
- **Calls through a local alias** `local f = outer; f()` → recorded as bare `f`, unresolved (external bucket, `no-candidate-in-file`; fixture p1 line 52).
- **Dynamic index calls** `_G["dynamic"]()`, `t[k]()` → the whole text is recorded as the call name (`_G["dynamic"]`) with empty receiver (lua.rs:281, since the callee is a `bracket_index_expression`). Fixture p1 line 59: external bucket, name `_G["dynamic"]`.
- **Registrar arguments**: none captured. CHANGELOG.md:5056-5058 records that Lua "was assessed and deliberately **not** changed" because its two framework rules are gap rules.
- **Arity** not recorded (Lua has no overloads; not needed by any narrowing).
- **Explicit generics / operator calls / call-operator (`__call`)**: not applicable or not modelled; `call_operator_names` has no Lua entry (cross_file.rs:394-400).
- **Method references, paren-less calls**: not applicable to Lua syntax.

## 04. Imports & module resolution

Supported:

- **`require` forms**: `local m = require("a.b")`, `m = require "a.b"`, `require('a')`. The argument's first named child is unquoted (`'`/`"`) as the path (lua.rs:292-298); the alias is the identifier the call is assigned to when the call is the whole value at that position (`require_binding`, lua.rs:192-205). Unit test asserts `("util","util")`. CHANGELOG.md:5037-5040 records the fixed bug where every require was recorded as `"("`.
- **Module file index** (`ModuleIndex::lua_by_last`, modules.rs:183-186, 253-265): each Lua file's path segments become its module path; `a/b.lua` and `a/b/init.lua` are both `a.b`. `lua_files` (modules.rs:1282-1306) returns every file whose module path ends with the require's dotted (or `/`-separated) segments.
- **Calls through a require-bound name** (`resolve_lua`, modules.rs:1650-1708; dispatched from cross_file.rs step 0c, cross_file.rs:5388): `util.f()` / `util:f()` binds only to a member of a single-level table in the required file (`lua_members`, modules.rs:1308-1330: qn `X.f` or `X:f`, owner without `.`/`:`). Fixture p1: `util.decode` → `M.decode` **high**, `util:encode` → `M:encode` **high**, `pkg.run` (`require "lib.pkg"` → `lib/pkg/init.lua`) → `P.run` **high**. Test `cross_file::tests::lua_calls_through_a_require_bind_in_the_required_file_only` (cross_file.rs:11736).
- **Vendored copies**: several files providing the module narrow to the one sharing the longest directory prefix with the caller; a tie is `Scoped::Ambiguous` (modules.rs:1676-1707).
- **External-module refusal**: a require naming no file in the tree → `Scoped::External(module)` → `external-module` (modules.rs:1668-1670). Fixture p1 `cjson.encode` → external bucket, `{'stage': 'external-module', 'detail': {'module': 'cjson'}}`.
- **Not-in-imported-module refusal**: the module's file exists but provides no such member → `Scoped::Refused` (modules.rs:1692-1694). In the unit test this is asserted as `refused("util")`; in fixture p1 `util.missing()` lands in the external bucket with reason `no-candidate-in-file` (the CLI audit did not show `not-in-imported-module`; the reconciliation step may rewrite it — unverified which).
- **Conflicting requires**: a local name bound by two different requires is dropped from the map (modules.rs:1434-1450).
- **Locals shadow the receiver head**: step 0c is skipped when `heads.locals.shadows_receiver_head(r)` (cross_file.rs:5389).
- **Legacy include-style reach** (cross_file.rs:1846-1866): a `require` import also makes every definition of a file at `path`, `path.lua` or `path.replace('.', '/') + ".lua"` reachable for the older name-based steps; the module alias is also fed to the step-1c scoped lookup as a path fragment (cross_file.rs:2208-2211).

Not supported / partial:

- **Deeper path through a module** `util.sub.deep()` → `resolve_lua` returns `None` when `rh != head` (modules.rs:1671-1673), so the site falls through to by-name matching. Fixture p1: `util.sub.deep()` → `M.sub.deep` **medium** via `cross-file:imports` (correct target, by name).
- **Module returning a literal table** — nothing to bind (see §02); calls are external/unresolved.
- **`package.path` / `package.cpath`, `package.loaded`, `package.preload`, `dofile`, `loadfile`** — not read (only `require` is matched, lua.rs:287).
- **Global modules** (a module that sets a global table instead of returning one, e.g. `module("x", package.seeall)`, or `_G.M = M`) — no binding; calls reach it only by name.
- **`FileFacts::packages`**: never set for Lua (grep: no `packages` in lua.rs).
- **Manifest reading**: none (no rockspec / `.luarc.json` reading; `plugins/manifest.rs` not used).
- **`heads.rs` `WorkspaceVocab` / `FileHeads`**: Lua is not one of the import-table languages listed there (grep for `lua` in heads.rs finds nothing); Lua's external refusal comes solely from `modules.rs`.
- **Inline `require("x").f()`**: not resolved (see §03).

## 05. Type inference & receiver typing

- **None for Lua.** `type_hints.rs`, `closure_types.rs` and `receiver.rs` contain no Lua handling (grep for `lua` finds nothing). The plugin records no `local_types`, `arg_type`, `scoped_types`, `class_fields`, `typed_receiver`, `weak_receiver_type` or `implicit_receiver`.
- **Constructed locals are not typed**: `local a = Animal.new("x"); a:speak()` binds `Animal:speak` only by name, at **medium** (fixture p1 line 37, `cross-file:imports`).
- **`self` is not typed** to the method's table: `self:sound()` inside `Animal:speak` with `Animal:sound` and `Dog:sound` in the file → `ambiguous-in-file` plus `candidates-in-other-files` (fixture p1 line 16). Where only one same-named member exists in the file (`self:helper2()` in `M:encode`) it binds **high** by uniqueness (fixture p1 line 12).
- **Receiver narrowing by text**: intra-file keeps candidates whose qn segments (split on `:` and `.`) include the receiver hint (intra_file.rs:567-571), so `M.f()` narrows to `M.*` definitions in the same file. This is textual, not typed.
- **Casts/constructions/wrappers/nullability/`declared_type`**: not applicable (no type annotations in Lua; LuaLS `---@type` / `---@param` annotation comments are not read).

## 06. Inheritance, dispatch & overloading

- **Base-class records**: none. The plugin records no `classes`/`base_types`; `setmetatable(Dog, { __index = Animal })` is not interpreted. Fixture p1: `Dog` "inherits" `Animal`, but nothing links them.
- **super/base calls**: no Lua syntax; `Base.method(self, …)` is an ordinary dot call resolved by name.
- **`member_of_calling_class`, `overrides_below`, `resolve_via_bases`**: not reached for Lua (no owner/base facts).
- **`INHERITANCE_LANGS`** (dispatch.rs:110-123) does not include `lua`, so `inheritance_fanout` never runs; `--dynamic-dispatch` adds nothing (no `trait_impl_target`). Fixture p1 with `--dynamic-dispatch` → no `dyn` edges.
- **Overloading**: `OVERLOAD_ARITY_LANGS = ["java","scala","cpp"]` (type_hints.rs:1583); not applicable to Lua.
- **Mixins/traits/extension methods**: not modelled.
- **Bare call vs member**: a bare call never reaches a table member (`bare_call_is_never_a_method`, intra_file.rs:49-53; cross_file.rs:4137-4139 treats Lua like Python/JS: `!member`). Test `intra_file::tests::a_lua_bare_call_never_reaches_a_table_member` (intra_file.rs:1546).

## 07. Scoping & shadowing

- **`FileFacts::locals`** supported via `LUA_SPEC` (locals.rs:206-219): `parameters` names (`Scope::Param`), `variable_list` names of `assignment_statement` and `for_generic_clause` declared after their value (`Scope::Decl`), `for_numeric_clause` names (`Scope::Own`); function nodes `function_declaration`/`function_definition`; blocks `block`/`for_statement`. Tests `locals::tests::a_declaration_is_not_in_scope_in_its_own_initializer` (Lua case locals.rs:757) and `dart_ocaml_lua_python_rust_kotlin_swift_scala_zig` (locals.rs:840, Lua case 865: binds `a,b,k,i,v`, not `f,g,t`). Skipped for minified files (§01).
- **shadowed-by-local**: a bare call through a parameter/local is never another file's function (pipeline-wide rule). Fixture p1 `local f = outer; f()` → unresolved, not bound elsewhere.
- **Lexical narrowing** of nested definitions: `lexically_scoped` includes `lua` (intra_file.rs:120-136). Fixture p2: `p → h` (line 16) and `q → h` (line 17) each bind their own nested `h` at high.
- **Positional shadowing**: Lua is in the OCaml/F#/Lua rule — among several same-scope candidates the last one declared before the site wins (intra_file.rs:400-416). Fixture p3: `g → f@1` and `h → f@3`, both high.
- **Gap — use before a local declaration inside the same function**: lexical narrowing prefers the innermost definition before the positional rule runs, and does not check that the nested definition is declared before the site. Fixture p3: in `run`, `outer()` at line 7 (before `local outer = function` at line 8) binds to the line-8 local at **high**; the correct target is the file-level `outer` (line 5). Same in p1 (line 47 and the `cb` closure at line 49 both bind the later local `outer`), which then makes the real `outer` (and its `inner`) a dead-code finding.
- **Conditional definitions** (`if x then function f() end else function f() end end`): no `DefRecord::conditional` for Lua; both definitions are recorded (inferred from code; not run).
- **Global vs local functions**: not distinguished in visibility (both empty, lua.rs:226, 247).

## 08. Macros, preprocessor & codegen

- Not applicable: Lua has no macros or preprocessor. No `macro:<name>` tagging, no `name_mentions` (grep: neither appears in lua.rs). `load`/`loadstring` of generated code is not followed (§15).

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: no Lua coverage (grep for `lua` in `crates/cgg-resolve/src/ffi.rs` finds nothing). LuaJIT FFI (`ffi.cdef[[ ... ]]` then `ffi.C.fn()`), the C API (`lua_pushcfunction`, `luaL_Reg` tables, `luaopen_*` exports) and Neovim's `vim.fn`/`vim.api` bridges are not linked. A C `luaopen_x` is not tied to `require("x")`.
- **Descriptor links**: not applicable.
- **Markup code-behind**: not applicable.

## 10. Frameworks & entry points

Rules with `language: "lua"` in `crates/cgg-core/src/frameworks/rules.rs` (no rule lists several languages; every `RuleSpec` has a single `language` field, rules.rs:28-31):

| rule id | kind | trigger | enumerates? | evidence |
| --- | --- | --- | --- | --- |
| `kong` | `TrustKind::Network` | import prefix `kong` (`detect: &["kong"]`) | No — declared gap; no attributes/registrars/base_types/methods | rules.rs:3124-3140 |
| `openresty` | `TrustKind::Network` | import prefix `resty` or `ngx` | No — declared gap | rules.rs:3141-3157 |

- Fixture p2 (`require "kong.plugins.base_plugin"`, `require("ngx")`) with `--framework-coverage` → both listed as "found in 1 file(s), entries NOT enumerated" with their gap text. No entry nodes minted. `crates/cgg/tests/detect_prefixes.rs:48` synthesises `local _x = require("<prefix>")` for every Lua detect prefix.
- **Stale gap text**: the Kong gap says the plugin "records each of these as ONE callable whose simple name is the whole `XHandler:access`". Since the WS-era change (CHANGELOG.md:863-868) the simple name is the last segment (`access`; fixture p2 node `XHandler:access | access`), so a `methods = ["access", …]` rule could now match; the gap's claim that "anonymous functions in table fields are not recorded" is still true for Kong's nested `["/path"] = { GET = function … }` route tables (fixture p1: no `GET` node).
- `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for`, `self_module_markers_for`: no Lua arms (rules.rs:11666-11865).
- **Registrar verbs**: `registrar_verbs_for("lua")` returns `None` (no Lua rule has registrars, rules.rs:11929-11955).
- **Frameworks with no rule at all** (unsupported): LÖVE (`love.load/update/draw` callbacks), Neovim plugins (`setup()`, autocmd/keymap callbacks), Lapis, Busted, Defold (`init/update/on_message`), Roblox/Luau, Corona/Solar2D, WoW addons, Hammerspoon, AwesomeWM, Redis scripts, Pandoc filters.

## 11. Test harness entries

- **No `kind: test` rule for Lua** (no Busted, LuaUnit, Telescope or `plenary.busted` harness). Busted's `describe("…", function() it("…", function() … end) end)` produces no callables (anonymous functions), so calls inside are `no-enclosing-callable` (fixture p1 `spec/util_spec.lua`: `describe`, `it`, `util.decode` all `src: None`).
- **Name-based test roots**: `is_test_name` default arm — names starting `test_` or `Test` (roots.rs:975-990). Fixture p1: `test_something` → root `builtin:test-name` (`test-entry`). This is LuaUnit-shaped (`function TestX:testFoo()` simple name `testFoo` does **not** match: lowercase `test` without underscore; inferred from the code).
- **Test-file classification**: no Lua-specific file-name rule (`_spec.lua` / `_test.lua` absent from `crates/cgg-core/src/testfile.rs` by-name table); only the generic `TEST_DIRS` (`test`, `tests`, `spec`, `specs`, …) apply. Fixture p1 `spec/util_spec.lua` → `test_role: directory`. A `foo_spec.lua` outside such a directory is production code.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** = `["python","php","ruby"]` (rules.rs:11806): Lua not included.

## 12. Dead-code analysis

- **Capability class**: Lua is `Degraded` (no visibility and no attributes recorded; caps.rs:163-172), so `max_confidence: medium` — no Lua finding is ever `high`. Fixture p1 `--dead-code`: `capabilities: class degraded, visibility none, attributes none, value_references none, dispatch none, exports none, test_tagging none, max_confidence medium, root_rules_active ["builtin:test-name","toplevel:invocation"]`; "0 shown (high) · 9 withheld (medium) · 2 withheld (low)". Every Lua finding carries `language-lacks-attributes/dispatch-model/value-references/visibility` evidence.
- **Root kinds that apply**:
  - `toplevel:invocation` — a callable whose simple name matches a call at file top level (roots.rs:552-614). Fixture p1: `run()` at main.lua:70 roots `run`. **Gap**: the match is by `(language, simple_name)` across the whole tree, so the same top-level `run()` also roots the unrelated `P.run` in `lib/pkg/init.lua` (fixture p1 root list). **Gap**: only intra-file `NoEnclosingCallable` records count, so a top-level call through a module (`util.decode()` in a spec file, external bucket) roots nothing.
  - `builtin:main` — a callable named `main` (`UNIVERSAL_ENTRY`, roots.rs:20); no Lua-specific `LANG_ENTRY`/`LIFECYCLE` entry.
  - `builtin:test-name` — §11.
  - `builtin:lifecycle` dunders: Lua is in `DUNDER_LANGS` (roots.rs:160-161), but `is_dunder` requires a `__` prefix **and** suffix (roots.rs:967-969), so Lua metamethods (`__index`, `__call`, `__tostring`, `__gc`) never match. `CONVENTIONAL_METHODS` (`initialize`, `to_s`, …, roots.rs:162-170) applies by name.
- **Not roots (gaps)**: the table a module `return`s is not an export (no `ExportRecord`s; `exports: none`), so a library's public API with no in-tree caller reads as dead; global functions are not public; framework callbacks (LÖVE, Neovim, Kong phases) are not rooted.
- **Confidence caps**: `implements-external-contract`, `macro-generated` and `reflection` never apply (no overrides, macros or dynuse for Lua). Unresolved same-name sites lower findings to `low` (`ambiguous-site-in-same-file` evidence; fixture p1 `Animal:sound`, `Dog:sound`).
- **False findings observed in fixture p1**: `run2` (passed to `pcall`), `dynamic` (called via `_G["dynamic"]`), `cb` (stored in a local), `outer`/`inner` (mis-bound, §07).

## 13. Builtins & stdlib tables

- `crates/cgg-core/src/stdlib/lua.txt`: 123 lines, ~122 names (`abs` … `yield`), loaded as `("lua", parse(LUA))` (stdlib.rs:200, 241). Names are bare members and module tables (`string`, `table`, `math`, `io`, `os`, `coroutine`, `debug`, `utf8`, `package`) plus globals (`print`, `pairs`, `ipairs`, `pcall`, `xpcall`, `setmetatable`, `require`, `load`, `dofile`, `getfenv`/`setfenv`, `unpack`, bit ops `band/bor/bxor/bnot/lshift/rshift`).
- Fixture p1: `setmetatable`, `print`, `string.format`, `table.insert`, `pcall`, `select` → `stdlib_calls` (`name_screen_applied: stdlib`); `assert.are.equal` (luassert, fixture p1 spec file, receiver `assert.are`) is also screened as stdlib although `equal` is not in lua.txt (`assert` is). Which screen rule fired was not checked (unverified).
- **Missing names**: `loadstring` (Lua 5.1/LuaJIT) is absent → fixture p1 `loadstring` goes to `external_calls`. `bit32.*`, LuaJIT `bit.*`/`ffi.*`, `jit.*`, `ngx.*`, `vim.*` are not stdlib (`ngx.say` → external, fixture p2).
- `external.rs:192-200` splits a dotted bare name (`table.insert`) on the last `.`/`:` so receiver rules apply uniformly.
- `--include-stdlib` / `--include-external`: mint `<stdlib>::print`, `<stdlib>::string::format`, `<external>::cjson::encode`, `<external>::util.sub::deep`, `<external>::_G['dynamic']` etc. (fixture p1 mermaid).
- No `rust_types`-like table, no `CAST_INTRINSICS`/`CONSTRUCTING_METHODS` Lua entries, no implicit packages (grep).

## 14. Confidence & certainty

- **high** observed for: intra-file single-candidate bare calls (`run → helper_str`, `outer → inner`), intra-file receiver-narrowed member calls (`M.decode → M.helper`), the positional/lexical picks (§07), and require-bound module members (`resolve_lua` returns `Scoped::Bound { certain: true }`, modules.rs:1702-1705; fixture p1 `util.decode`, `util:encode`, `pkg.run`, all `cross-file:imports` high).
- **medium**: by-name cross-file matches with an untyped or deeper receiver (`a:speak()` → `Animal:speak`, `util.sub.deep()` → `M.sub.deep`, `obj.a.b:c()` in fixture p2). Mermaid marks these with `?` (fixture p1: `N15 -->|?| N9`, `N15 -->|?| N5`).
- No typed-receiver (`OwnerFacts`) path exists for Lua, so a member call through a variable is never high unless the name is unique in the calling file.
- **False high** observed: the use-before-local-declaration mis-binding (§07) is emitted at high.
- `unique_owner_names` treats Lua owner paths as per-file (cross_file.rs:4391).

## 15. Dynamic use & reflection

- **dynuse**: Lua has no entry in `REFLECTIVE` (plugins/dynuse.rs:26-57; Python, Ruby, Java, C#, JS, TS, PHP, Go only). Nothing caps a Lua finding for reflection.
- `_G["name"]()`, `_G[name]`, `rawget(_G, "f")`, `M[method](...)`, `load("return f()")`, `loadstring`, `dofile`, `setfenv`, `debug.getinfo` — none produce edges or doubt signals. Fixture p1: `dynamic` (called only through `_G["dynamic"]`) is a medium dead finding with no reflection evidence.
- Module loads: `DYN_MODULE_LOADERS = ["require","importScripts","import_module"]` (facts.rs:165) is consumed only by languages with dynuse capture; Lua's `require` is handled as an import instead.
- `__index`/`__call` metamethod dispatch is not modelled (§06).

## 16. Known gaps & limitations

- **Stale `gap_for` text** for `kong` (rules.rs:3137): simple names are now split (`access`), so a phase-method rule is now possible; still has no enumeration. `openresty` gap (rules.rs:3154): nginx `*_by_lua*` directives are not parsed.
- **CHANGELOG follow-ups**: CHANGELOG.md:1349-1350 ("Not covered (WS14) … Lua module tables (`function M.f` is not extracted, WS8)") — superseded by CHANGELOG.md:863-868 which split names and added module binding; module tables returned as literals remain unextracted (fixture p2 `ret.lua`). CHANGELOG.md:5056-5058: registrar capture deliberately not added.
- **Unsupported constructs found while cataloguing** (all fixture-confirmed unless marked):
  1. Anonymous functions are not callables (Busted tests, callbacks, route tables) → calls inside them at top level are orphaned.
  2. `return { f = function() end }` modules produce no callables.
  3. Use of a name before a later `local` of the same name in the same function binds to the later local at high (lexical narrowing ignores declaration order).
  4. No `self`/constructor typing; `setmetatable` inheritance invisible; chained calls on returned objects unresolved.
  5. Value references (`pcall(f)`, `table.sort(t, cmp)`, `local g = f`) never recorded → false dead findings.
  6. Nested function qns carry no enclosing scope (`h` and `h`), since `scope` is never pushed (lua.rs:37, 54-60).
  7. `toplevel:invocation` roots every same-simple-name callable tree-wide (`run()` roots `P.run`).
  8. Metamethod dunders never root (is_dunder needs `__` suffix).
  9. Inline `require("m").f()` and deeper `m.sub.f()` are not module-scoped.
  10. No Lua dynuse, no visibility (`local function` ≠ private), no exports, no test harness, `loadstring` missing from stdlib.
  11. `.luau`, `.nse`, `.rockspec` not detected.
- **Tests that are weak**: `extracts_references` (lua.rs:395-400) asserts nothing ("just verify extraction works"). No ignored Lua tests (grep `#[ignore]` near Lua tests finds none; unverified across all crates).

## Summary scorecard

| section | support | one-line note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.lua` + `lua`/`luajit` shebangs; no `.luau`/`.nse`; depth/minified gates apply |
| 02. Callables extracted | partial | named, local, table-field, colon, assigned and named-table-literal functions; anonymous fns and returned literal tables missing; nested qns unscoped |
| 03. Call-site forms | partial | bare/dot/colon/string/table-arg calls; no value refs, no arity, chained/dynamic-index calls unresolved |
| 04. Imports & module resolution | partial | `require` → file via module path incl. `init.lua`, high binding, external refusal; no deeper paths, inline require, package.path |
| 05. Type inference & receiver typing | none | no locals/self/constructor typing; textual receiver narrowing only |
| 06. Inheritance, dispatch & overloading | none | no metatable inheritance, not in INHERITANCE_LANGS; bare calls never reach members |
| 07. Scoping & shadowing | partial | LUA_SPEC locals, lexical + positional shadowing; use-before-local mis-binds at high |
| 08. Macros, preprocessor & codegen | n/a | Lua has no macros; load/loadstring not followed |
| 09. FFI & cross-language links | none | no LuaJIT FFI / C API / luaopen links |
| 10. Frameworks & entry points | minimal | `kong`, `openresty` detect-only gap rules; kong gap text stale |
| 11. Test harness entries | minimal | only `test_`/`Test` name roots and `spec/` dir tagging; no Busted/LuaUnit |
| 12. Dead-code analysis | minimal | Degraded class, max medium; no exports; false roots/findings from name-keyed top-level rule and missing value refs |
| 13. Builtins & stdlib tables | partial | 122-name lua.txt; `loadstring`, `bit32`, LuaJIT/ngx/vim missing |
| 14. Confidence & certainty | partial | high for intra-file and require-bound members; medium + `?` for by-name; one false-high path |
| 15. Dynamic use & reflection | none | no dynuse for `_G[...]`, `load`, `rawget` |
| 16. Known gaps & limitations | n/a | 11 gaps listed above |
