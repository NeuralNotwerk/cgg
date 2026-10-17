# zig

The Zig plugin (`crates/cgg-lang/src/plugins/zig.rs`, 830 lines, `ZigPlugin`, id `"zig"`) is a hand-written tree-sitter walker over the `tree-sitter-zig` crate (`Cargo.toml:103`, `tree-sitter-zig = "1.1"`, bound as `tree_sitter_zig::LANGUAGE`, zig.rs:18-20; nothing is vendored). It extracts every named `function_declaration` as a `DefVariant::FreeFunction` whose qualified name is the bare function name (zig.rs:158-210), records every `call_expression` as a `RefRecord` split on the last `.` (zig.rs:245-266), turns `@import("x")` into `ImportRecord { kind: "import" }` with the bound name as alias (zig.rs:267-305), records file-level member aliases (`alias`, `import-member`, zig.rs:600-648), records container spans (`FileFacts::type_spans`, with container-level `decls`) and `@This()` aliases (zig.rs:211-225, 335-418), and types function locals from what is written or what a same-file function returns (`local_types`, zig.rs:424-597). A `build.zig` additionally yields `zig-module` import records from `addModule(...)` (zig.rs:42-49, `manifest.rs:88-133`). There is no `resolver_kind` method any more; Zig goes through the shared pipeline plus a dedicated Zig library-scope resolver in `crates/cgg-resolve/src/libscope.rs` (`build_zig` :1233, `zig_target` :1385, `resolve_zig` :1449, `zig_member_call` :1529, `zig_alias_call` :1574, `zig_generic_call` :1628, `zig_type` :1774, `zig_walk` :1836), called from `cross_file.rs:5455-5476` (WS14), plus Zig-only branches in `cross_file.rs` (alias follow :5777-5820, own-file bare-name guard :5974-5988, typed-receiver read-as-written fallback :2525) and `type_hints.rs` (scoped local lookup :419, param type peel `zig_peel_type` :2709, `untyped_receivers` :933). `ExtractCtx` is ignored (`_ctx`, zig.rs:24); the plugin does not override `signals()` (default `PluginSignals::default()`, `cgg-lang/src/lib.rs:304-306`), so it declares no visibility, attributes, exports, test defs, value refs or dyn uses.

Fixtures used for this document live in `<scratch>/parity/zig/` (`p1/` general shapes, build.zig modules, tests, exports, dead code; `p2/` skips, parse errors, payloads, vtables, local structs; `p3/` typed receivers, an in-tree `lib/std/std.zig`, duck fan-out, shadowing; `p4/` same-named members of several containers in one file; `p5/` Zig/C FFI), run with `cgg 0.11.0` (`target/release/cgg`). `show.py` / `sites.py` there dump edges and per-site audit reasons.

## 01. Detection & parsing

- **Extension** `.zig` only (zig.rs:15-17). Fixture p1: every file `detected_via: extension:.zig`.
- **Shebangs**: none declared (`shebangs()` not overridden). Fixture `p2/script` (`#!/usr/bin/env zig run`, no extension) → `file_skipped … unknown-extension`.
- **Content detection**: none (no Zig entry in `crates/cgg-lang/src/detect.rs`; grep for `zig` there finds nothing).
- **Not detected**: `.zon` (ZON manifests, `build.zig.zon`) → fixture `p2/build.zig.zon` `file_skipped … unknown-extension`; `libscope.rs:1388` does accept a `.zon` *import spec* when resolving `@import("x.zon")` but such a file is never analysed, so the target is `Unknown`.
- **Grammar**: single grammar `tree_sitter_zig::LANGUAGE` (zig.rs:18-20); no `ts_language_for` override.
- **Re-parse / fallback**: none. A file with errors is extracted from the error-recovered tree; `parse_status: "error"` (`crates/cgg/src/lib.rs:457-458`). Fixture `p2/err.zig` (`fn broken( void { callMe(); } fn callMe() void {} fn ok() …`) → status `error`, callables `broken`, `ok` only: `callMe` was swallowed by recovery and both `callMe()` sites went to the external bucket.
- **Depth skip**: tree deeper than `MAX_TREE_DEPTH` (4000) → `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:451-454`). Fixture `p2/deep.zig` (5000 nested parens) → `file_skipped {'kind': 'too-deep', 'detail': 4000}`.
- **Long-line skip**: `max_line_bytes()` not overridden → no `LongLine` cap.
- **Minified files**: any line > 2,000 bytes (`MINIFIED_LINE_BYTES`, `crates/cgg/src/lib.rs:159`) disables `locals::collect` (lib.rs:480-487); extraction proceeds. Fixture `p2/min.zig` (one ~5 KB line, 600 calls) → analysed, `parse_status ok`, 600 external sites.
- **Build scripts**: a file named exactly `build.zig` is additionally scanned textually for `addModule("<name>", .{ … .root_source_file = b.path("<path>") … })` (zig.rs:45-49, `manifest.rs:93-133`); a computed (non-literal) root is skipped (`manifest.rs:88-92`). Unit test `import_names_and_build_modules` (zig.rs:747).

## 02. Callables extracted

Every callable is a `function_declaration` with a `name` field → `DefRecord { simple_name: name, qualified_name: name, variant: FreeFunction, signature_hint: extract_signature(text), visibility: "", attributes: [] }` (zig.rs:158-203). Kind in output: `function` (fixture p1, all 30 nodes).

Supported:

- **File-level function** `fn add(a: i32, b: i32) i32 { … }`, `pub fn main() !void {}` → `add`, `main`. Unit test `functions_extracted` (zig.rs:701).
- **Methods of a named container** `pub const Counter = struct { pub fn inc(self: *Self) void {…} };` → callable `inc` with qn `inc` (no owner prefix). Membership is recorded separately: the container's span goes into `FileFacts::type_spans` (`TypeSpan { name: "Counter", … }`, zig.rs:335-360) and `build_zig` assigns each callable to the innermost span around it (`zig_members`, libscope.rs:1300-1318). Fixture p1 `lib.zig`: `init`, `inc`, `bump`.
- **Nested containers** `pub const Inner = struct { pub fn deep() void {} };` inside `Counter` → `deep`, span path `Counter.Inner` (zig.rs:344-348). Fixture p1 `Counter.Inner.deep()` → edge high.
- **Union / enum / opaque methods** (`CONTAINERS`, zig.rs:121-126): `const Shape = union(enum) { pub fn area(…) … }`, `const Color = enum { red, pub fn name(…) … }` → `area`, `name`. Fixture p1.
- **File-as-struct** (`const Self = @This(); x: u32, pub fn init() Self …` at file level): functions are file-level; the `@This()` alias is recorded in `this_aliases` with key `u32::MAX` (zig.rs:404). Fixture `p2/file_struct.zig` → `init`, `get`; `fs.get()` with `fs: *FS` (`FS = @import("file_struct.zig")`) → high.
- **Anonymous containers** (`return struct { pub fn push(…) … }` in a type function `fn List(comptime T: type) type`) → `push`, `init` callables, member of an unnamed span (zig.rs:211-224). Fixture p1 `List`, `init`, `push`.
- **Containers declared inside a function** (`const Local = struct { fn inner() void {} };`) → `inner` extracted, span recorded with an empty path (zig.rs:342-345). Fixture `p2/use.zig` `inner`; but calls to it do not resolve (§03).
- **Generic / comptime functions** `fn generic(comptime T: type, v: T) T` → ordinary callable. Fixture p2 `generic` (edge high).
- **`inline fn`, `export fn`, `pub fn … callconv(.c)`** → ordinary callables, no marker recorded. Fixture p1 `inl`, `c_entry`, `callc`.
- **`extern fn puts(...) c_int;`** (bodiless prototype) → extracted as a callable (it is a `function_declaration` with a name). Fixture p1 `puts`, fixture p5 `c_func` (and a same-file call binds to the prototype at `high`, §09).
- **Signature hint** is the declaration text up to the body (`extract_signature`), which `build_zig` reads for the return type (`zig_return_type`, libscope.rs:2348; test `zig_return_types_are_peeled` :2394) and to mark type functions (signature ending in `type`, libscope.rs:1275-1277).

Not supported / partial:

- **Qualified names carry no owner**: two containers in one file each with `helper`, plus a file-level `helper`, are four callables all named `helper` (fixture p4). Mermaid/JSON labels cannot tell them apart; only spans distinguish them.
- **`test "name" { … }` / `test { … }`** → no callable (`test_declaration` is not walked as a definition). Fixture p1: two test blocks, no nodes; calls inside are `no-enclosing-callable`. Also stated in the `zig-test` rule gap (`rules.rs:6608`).
- **`comptime { … }` blocks** → no callable; calls inside have no enclosing callable (fixture p1 `@export(&local, …)` line 74 region).
- **Function-typed constants / fields** (`const f = local;`, `write: *const fn (*anyopaque) void` vtable fields) → not callables. Fixture p1 `f`, p2 `VTable.write`.
- **Anonymous functions**: Zig has no lambdas; n/a.
- **No visibility** (`pub` is not recorded: `visibility: String::new()`, zig.rs:200), **no attributes** (`export`, `extern`, `inline`, `callconv` not recorded, zig.rs:201). Confirmed by the `zig-export` rule gap (`rules.rs:6625`) and fixture p1 (all empty).
- **Error-recovered regions** can lose definitions (fixture `p2/err.zig`, `callMe`).

## 03. Call-site forms

Every `call_expression` with a non-empty `function` text becomes one `RefRecord { name: text after last '.', receiver_hint: text before it }` (zig.rs:245-266). No arity, no `typed_receiver`, `path_receiver`, `value_ref`, `paren_less`, `arg_type` flags are set by the plugin.

Supported:

- **Bare call** `local();` → `local`, empty receiver; intra-file binds at high. Fixture p1 lines 25/34/35/44.
- **`defer f()` / `errdefer f()`** → ordinary calls. Fixture p1 lines 34-35 → `cleanup` high (twice).
- **Calls in `inline for`, `switch` prongs, `catch |err|` blocks** → recorded normally. Fixture p2 lines 46-48 → `target`, `brokenCall` high.
- **Module path call** `std.debug.print(…)`, `lib.libEntry(2)`, `mylib.libEntry(1)` → receiver `std.debug` / `lib` / `mylib`, resolved through `resolve_zig` (§04). Fixture p1 lines 13-14 high; fixture p3 `std.debug.print` → in-tree `lib/std/debug.zig:print` high.
- **Container-qualified call** `Counter.init()`, `Counter.Inner.deep()`, `Node.visit2(self)`, `A.helper()`, `Self.helper()` → resolved as a member of the named container (`zig_member_call`). Fixture p1 lines 15/21 high, p2 line 17 high, p4 lines 7-8/16 high.
- **Method call on a typed value** `c.inc()`, `x.inc()` → receiver rewritten to the type by type propagation then resolved as a container member (§05). Fixture p1 lines 16/18/20/37/39, p3 lines 10-11, 17 high.
- **Call on a call result / generic instantiation** `lib.List(u8).init()` → receiver `lib.List(u8)`; `resolve_zig` sees `(` and calls `zig_generic_call` (libscope.rs:1454-1456), which binds the member of the struct the type function returns at **medium** (`certain: false`, libscope.rs:1765-1768), or follows a plain function's declared return type at high (libscope.rs:1690-1726). Fixture p1 line 22: `init` medium, plus `List` itself high.
- **Calls written through an alias** `const helperAlias = util.helper; helperAlias(3)` → followed (cross_file.rs:5777-5820), high. Fixture p1 line 24.
- **Calls through a re-export** `pub const helper = @import("util.zig").helper; helper(x)` → medium. Fixture p1 `lib.zig:6` (`util.helper(x)` high, `helper(x)` medium).
- **Explicit type arguments**: Zig passes types as ordinary arguments (`generic(u8, 3)`), recorded as a plain call. Fixture p2 line 52 high.

Partial / not supported:

- **Field-chain receivers** `self.vtable.write(self.ptr)`, `obj.c.inc()`, `h.ctr.inc()`, `self.child.?.visit()` → no field typing; these fall to name matching (duck fan-out at medium when the name exists in the tree, e.g. p1 line 41 `obj.c.inc()` medium, p3 line 19 two medium edges) or the external bucket (p2 line 8 `write`, line 15 `visit`). Listed as not covered in CHANGELOG.md:705-706 ("Zig receivers that are field chains").
- **Payload captures** `if (o) |oo| oo.inc()` / `if (self.child) |c| c.visit()` → payload is recorded as a local (`locals.rs:373`) but not typed: p3 line 12 duck fan-out (2 medium edges), p2 line 14 external.
- **Pointer deref receiver** `self.*.visit2()` → receiver `self.*`, medium by name (p2 line 16).
- **Function pointers / parameters called** `cb()` (`cb: *const fn () void`), `target2()` → no callable of that name; external bucket (p1 line 50, p2 line 27). When a same-named function exists the parameter shadows it (`shadowed-by-local`, p3 line 7).
- **Function used as a value** `const f = local; f();`, `callback(local)`, `@call(.auto, local, .{})`, `@export(&local, …)` → no reference edge for `local` from these sites; `f()` → external (p1 line 28). The plugin records no value refs (`signals()` default).
- **Builtin calls** (`@call`, `@field(obj, "method")()`, `@cImport`) → `builtin_function` nodes are not `call_expression`s and are not recorded, except that `@field(obj, "method")()` is the callee of an outer `call_expression` whose whole text becomes the name: external site named `@field(obj, "method")` (p1 line 42).
- **`@cImport` members** `c.printf(…)` → external (p2 line 45).
- **Container declared inside a function** `Local.inner()` / `S.helper()` → external + `candidates-in-other-files` (p2 line 36, p4 line 31): the span has an empty path (zig.rs:342-345) so `zig_type` cannot name it.
- **Bare call of a same-named member inside its own container** `helper()` inside `struct A` when `B` and file scope also declare `helper` → `ambiguous-in-file`, no edge (fixture p4 lines 6, 15, 21, 32). Zig has no lexical narrowing entry (`intra_file.rs:119-135`).
- **Enum-literal value chain** `Color.red.name()` → external + `candidates-in-other-files` (p1 line 33).
- **Container-level calls** `pub const frame = Functions(@This()).frame;` → the `Functions(...)` call has no enclosing callable (p3 `types.zig:17`).
- **Arity**: not recorded (Zig is not in `OVERLOAD_ARITY_LANGS`, `type_hints.rs:1583`).
- **Registrar arguments**: none (no registrar capture; `registrar.rs` has no Zig entry).

## 04. Imports & module resolution

Import records produced (zig.rs, manifest.rs):

- **`const NAME = @import("x");`** → `ImportRecord { kind: "import", path: "x", alias: "NAME" }` (zig.rs:267-305). An `@import` not bound by a `variable_declaration` gets an empty alias. Unit tests `calls_and_imports` (:722), `import_names_and_build_modules` (:733).
- **`const NAME = @import("x.zig").a.b;`** (file-level or in a function) → `kind: "import-member", path: "x.zig#a.b"` (zig.rs:600-622, `import_member` :56-66). Test `import_names_and_build_modules`. Function-local form: zig.rs:236-241 (CHANGELOG.md:678).
- **`const NAME = a.b.c;`** at file level (plain dotted path, ≥2 segments) → `kind: "alias", path: "a.b.c"` (zig.rs:623-647). Test `a_member_alias_is_recorded` (zig.rs:709): `const n = link.items[0]` is not one.
- **`build.zig` `addModule("pkg", .{ .root_source_file = b.path("src/main.zig") })`** → `kind: "zig-module"` (manifest.rs:93-133).

Resolution (`LibIndex`, libscope.rs):

- **Relative file import** `@import("util.zig")` / `.zon`: resolved against the importer's directory (`zig_target`, libscope.rs:1388-1399). Fixture p1 `lib.libEntry` high.
- **Build-declared module** `@import("mylib")`: the nearest `build.zig` above the importer declaring it, else the single declaration (libscope.rs:1400-1423). Fixture p1 line 13 `mylib.libEntry(1)` → `lib.zig:libEntry` high. Test `zig_import_paths_and_build_modules` (cross_file.rs:13773).
- **Module rooted at a same-named file in a same-named directory** `@import("std")` → `lib/std/std.zig` (libscope.rs:1424-1438). Fixture p3: `std.debug.print` → `lib/std/debug.zig:print` high, `std.topStd()` high, `const assert = std.debug.assert; assert(true)` high.
- **External module**: a spec matching no file stem or directory name in the tree is `Target::External` (libscope.rs:1439) → the call is recorded in the external bucket under that module. Fixture p1 `ext.doThing()` with `ext = @import("zigextpkg")` → `unresolved_by_module` `zigextpkg`. An alias whose import base is not a tree package (`zig_packages`, cross_file.rs:1285-1297, :1524 adds build-module names) is external (cross_file.rs:5812-5819): `const assert = std.debug.assert` outside the Zig repo → stdlib/external, never a workspace `assert` (CHANGELOG.md:555-558; test `zig_alias_of_a_dependency_is_external_and_of_the_tree_is_followed`, cross_file.rs:13445). Fixture p1: `assert` → stdlib bucket.
- **Paths through several files** (`a.b.c.f()` through import → import → file) are walked by `zig_walk` (libscope.rs:1836-1880), following `alias` and `import-member` bindings; **re-exports** `pub const checksum = @import("vsr/checksum.zig").checksum;` are followed up to `MAX_DEPTH` (libscope.rs:1475-1525). Test `zig_member_aliases_and_own_declarations` (cross_file.rs:13206).
- **Copies of a module** (two `Type.zig`) narrow through the import alias (test `module_alias_copies_need_evidence`, cross_file.rs:12752).
- **Bare names stay in the file**: a bare name the calling file declares is never bound to another file (cross_file.rs:5974-5988, FINDINGS R13), and a bare call to a function of another file with no import does not bind: fixture `p2/caller2.zig` `onlyHere()` → `candidates-in-other-files` (correct for Zig, where an un-imported name is not in scope).
- **`not-in-imported-module`-style refusal**: Zig uses `Scoped::Refused` (cross_file.rs:5470-5473) for a container that declares nothing of the name, or a type-function result with no such member (libscope.rs:1544-1557, 1676-1771).

Not supported / partial:

- **`FileFacts::packages`**: not populated (no package/namespace notion beyond files); workspace membership is file stems + directory names (`zig_names`, libscope.rs:1322-1333; `zig_packages`, cross_file.rs:1285-1297).
- **`heads.rs` (`WorkspaceVocab`/`FileHeads`)**: Zig not covered (grep finds no `zig`); the external refusal is done in `libscope.rs`/`cross_file.rs` instead.
- **`usingnamespace`** (`pub usingnamespace @import("x.zig");`) → recorded only as an unaliased `import` record; members brought in are not visible as bare names (fixture p2; no special handling in code).
- **`@import("builtin")` / `@import("root")`** → external (no in-tree file): `root.customHook()` → external (fixture p4 line 23).
- **`build.zig.zon` dependencies** are not read (only `addModule` in `build.zig`); `b.dependency("x").module("y")` is not followed (no code path).
- **`addModule` with a computed root** is skipped (manifest.rs:88-92); `b.createModule`/`addImport` are not read (only `addModule(` is scanned, manifest.rs:104).

## 05. Type inference & receiver typing

- **Parameter types**: generic `collect_param_types` (type_hints.rs:1414) reads `name: Type`; for Zig every type is peeled of `?`, `*`, `const`, `volatile` (`zig_peel_type`, type_hints.rs:2709-2722; applied at :427-431). Fixture p3: `x: *Counter` → `x.inc()` high; `g: *types.Gauge` → `g.inc()` high.
- **Locals are scoped**, not file-wide: Zig local types are looked up where the declaration is in scope, as Rust's (type_hints.rs:416-419; `LocalType.scope_byte` = end of the declaration, zig.rs:428).
- **Written local type** `var c: ?*const Surface = null;` → `Surface` (`peel_type`, zig.rs:131-150, 436-446). Unit test `locals_are_typed_from_returns_and_annotations` (zig.rs:770).
- **Struct literal** `const d = Surface{ .id = 1 };` → `Surface` (zig.rs:473-478). Fixture p1 line 19-20 `c3.inc()` high.
- **Result of a same-file call** `self.active() orelse return`, `try self.me()`, `m()`, `T.init()` (peeling `try`, `catch`, `orelse`, zig.rs:449-470) → the one same-file function's peeled return type (`type_locals`, zig.rs:543-597), through `@This()` aliases (`type_from`, :519-537). Fixture p1 `v = maybe() orelse return; v.inc()` high; `w = try fallible(); w.inc()` high.
- **Result of another file's call** `const k = types.makeCounter();` → `local_types` `"types.makeCounter()"` (zig.rs:563-576), followed by the resolver through the callee's declared return type (`zig_generic_call`, libscope.rs:1690-1726). Fixture p3 line 16-17 `k.inc()` high; fixture p1 `var c = Counter.init(); c.inc()` high.
- **Container types**: `zig_type` names a type through file bindings to another file, then a container span (nested names tried from the site's surrounding containers first, libscope.rs:1774-1834); `@This()` returns map to the surrounding container or file (libscope.rs:1696-1708). Test `zig_container_typed_receiver_reaches_its_member` (cross_file.rs:13843).
- **Container-level aliases** `pub const frame = Functions(@This()).frame;` / `const C = Common(Self)` are recorded in `TypeSpan::decls` (zig.rs:361-382, `facts.rs:739`) and followed (`zig_alias_call`, libscope.rs:1574-1626). Fixture p3: `w.frame()` (w: `*types.Widget`) → `Functions(...).frame` at medium; `self.frame()` inside `Widget` → high intra-file.
- **Typed receiver the lookup cannot follow** is retried as written (cross_file.rs:2515-2560), using `facts.untyped_receivers` recorded for cpp/zig (type_hints.rs:933-941).
- **Duck-typed fallback**: an untyped receiver (`anytype` parameter, payload capture, field chain) fans out by name, bounded by `--fanout-cap`. Fixture p3: `y.inc()` with `y: anytype` → 2 medium edges; `--fanout-cap 1` → `fanout-cap-exceeded {candidates: 2}` on lines 12, 13, 19, 22.

Not supported:

- Field types (`self.child`, `obj.c`), payload captures (`|v|`), `anytype`, `comptime T` parameters, slices/arrays element types, generic instantiations (`ArrayList(u8)` values: `list.append` → external, fixture p2 line 41) — `peel_type` returns `None` for anything not a plain name/path (zig.rs:128-150).
- **Casts / constructions**: no Zig entries in `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` (stdlib.rs:35-65); `@as(T, x)`, `@ptrCast` receivers are untyped.
- `declared_type` tables (`MAYBE_TYPE_WRAPPERS`, `NULL_TYPE_MEMBERS`, `CALLBACK_TYPE_CONSTRUCTORS`, `TOP_TYPES`) have no Zig rows (stdlib.rs:104-160; grep finds no `zig`); Zig nullability is handled by `zig_peel_type` / `peel_type` instead.
- `closure_types.rs`, `receiver::peel`, `implicit_receiver`, `weak_receiver_type`: not used by the plugin (zig.rs imports none of them).

## 06. Inheritance, dispatch & overloading

- Not applicable in the class sense: Zig has no inheritance, interfaces, traits, `super`, extension methods or overloading. The plugin records no `ClassDecl`, no bases, no `trait_impl_target` (zig.rs writes none).
- **Interface-by-vtable pattern** (`self.vtable.write(self.ptr)`) is not modelled: field-chain receiver → external (fixture p2 line 8).
- **`usingnamespace` mixins**: not modelled (§04).
- Zig is not in `INHERITANCE_LANGS` (dispatch.rs:110-123), so `--dynamic-dispatch` / `inheritance_fanout` add nothing; fixture p1 with `--dynamic-dispatch --reference-edges` produced no `dyn`/`ref` edges.
- Not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583); no arity recorded. Zig forbids overloading, so no gap.
- `member_of_calling_class`, `overrides_below`, `resolve_via_bases`: not reached for Zig (no class records).

## 07. Scoping & shadowing

- **`FileFacts::locals`**: `ZIG_SPEC` (locals.rs:368-378) — binders `parameter` (name field, `Scope::Param`), `variable_declaration` (`Scope::Decl`), `payload` captures (`|x|`); functions `function_declaration`; blocks `block`, `for_statement`, `while_statement`, `if_statement`. Test `a_zig_same_named_member_alias_is_not_a_binding` (locals.rs:796) and the multi-language case table (locals.rs:907).
- **Shadowed-by-local**: a bare call through a parameter of a function name → `shadowed-by-local`. Fixture p3 `fn a(local: *const fn () void) void { local(); }` → `shadowed-by-local`.
- **Positional**: a local declared after the call does not shadow it (declaration in scope after its initializer). Fixture p2 `caller`: `target(); const target = 3;` → `target` high.
- **Same-named member alias is not a binding**: `const fatal = std.process.fatal;` in a function does not shadow (locals.rs:796-808, CHANGELOG.md:679).
- **Receiver head a local** is not an import (`shadows_receiver_head`, checked before `resolve_zig`, cross_file.rs:5456).
- **Own-file names first**: a bare name declared anywhere in the calling file is never bound to another file (cross_file.rs:5974-5988).

Not supported:

- **Lexical narrowing of nested definitions**: Zig is not in `lexically_scoped` (intra_file.rs:119-135), so a bare call inside a container is not narrowed to that container's member: fixture p4 → `ambiguous-in-file` for `helper()` in `A.run`, `B.run`, `top` and `outer`.
- **Conditional definitions** (`if (builtin.os.tag == .windows) struct {…} else struct {…}`, `comptime if`) → no `DefRecord::conditional`; both branches' functions are separate same-named callables (unverified beyond code reading: the plugin never sets `conditional`).
- Minified files lose locals (§01).

## 08. Macros, preprocessor & codegen

- Not applicable for macros/preprocessor: Zig has no macros or `#if`. `comptime` code generation (type functions returning `struct {…}`) is partly modelled: the returned anonymous struct's methods are callables and `zig_generic_call` binds `Fn(T).member()` to them at medium (libscope.rs:1727-1771).
- No `macro:<name>` tagging, no `name_mentions` (zig.rs never writes `name_mentions`).
- `@field(x, "name")`, `@call`, `inline for` over `std.meta.fields` (comptime reflection) are not modelled (§15).
- `@cImport(@cInclude("stdio.h"))` translated C is not seen (fixture p2 `c.printf` external).

## 09. FFI & cross-language links

- `ffi::link_ffi` has no Zig pass: asm↔C passes are C-family/asm only (ffi.rs:94-175), Kivy↔Python (ffi.rs:268), attribute-driven Pass B reads `DefRecord::attributes` (ffi.rs:191-260), which Zig never fills.
- **`export fn zig_export()` called from C** → no edge; C's call binds its own prototype intra-file (fixture p5: `b.c:call_zig -> b.c:zig_export high`).
- **`extern fn c_func(...)` called from Zig** → binds the Zig prototype node intra-file at high, never the C definition (fixture p5: `a.zig:run -> a.zig:c_func high`, `b.c:c_func` unreached).
- `@export(&f, .{ .name = "…" })` → not read (fixture p1, no root or edge).
- Descriptor links (protobuf rpc → host), markup code-behind: n/a for Zig.
- The `zig-export` rule documents the gap (rules.rs:6612-6628).

## 10. Frameworks & entry points

Rules whose `language` is `zig` (all four are detect-only: `attributes`, `registrars`, `base_types`, `methods` all `NONE`, so they never mint a node; they surface under "seen, no rules" in the coverage table). Fixture p1 coverage: `recognised: []`, `seen_no_rules: zig-build, zig-export, zig-main, zig-test`.

| rule id | kind | detect (import spec) | trigger / gap |
| --- | --- | --- | --- |
| `zig-main` | `Cli` | `std`, `builtin`, `root` | detect-only; gap text: `main` too common to match by name (rules.rs:6561-6576) |
| `zig-build` | `Cli` | `std`, `builtin` | detect-only; `build` not rooted (rules.rs:6578-6593) |
| `zig-test` | `Test` | `std`, `builtin` | detect-only; `test` blocks produce no callable (rules.rs:6595-6610) |
| `zig-export` | `Ffi` | `std`, `builtin` | detect-only; no visibility/callconv recorded (rules.rs:6612-6628) |

- No multi-language rule lists Zig (grep `"zig"` in rules.rs finds only these four).
- `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`: no Zig entries (no attributes are extracted, so none could fire).
- Registrar verbs: none. Visibility entries: none.
- Detection is verified by `crates/cgg/tests/detect_prefixes.rs:74` (writes `const _x = @import("<prefix>");`).
- Corpus apps (scripts/benchmark.sh:318-321): `app-ghostty-zig` (`~zig-build`), `app-libxev-zig` (`~zig-export`), `app-zls-zig` (`~zig-main`), `app-tigerbeetle-zig` (`~zig-test`) — `~` marks expected-unverified.
- `main` is rooted by the dead-code pass (`builtin:main`, §12), not by a framework rule; `build` in `build.zig` is not rooted (fixture p1 dead-code finding `build` medium).

## 11. Test harness entries

- `zig-test` (kind `Test`) is detect-only, so no `<framework-entry>::test::…` node is minted and no `entry` edge exists (fixture p1).
- `test "name" { … }` / `test { … }` blocks are not callables (§02); calls inside them are `no-enclosing-callable`. The dead-code pass then credits each called function as a **production** `toplevel:invocation` root (not a test root): fixture p1 `local` and `cleanup` → `RootKind::TopLevelInvocation` "called from module top level at main.zig:70/74" (roots.rs:570-612).
- Name-based test roots: Zig falls into the default arm of `is_test_name` (`test_*` or `Test*`, roots.rs:975-988): fixture p1 `test_underscore` → `builtin:test-name` test root; `testHelperName` → not rooted.
- Test-file classification: no Zig file-name rule in `testfile.rs` (only the generic test-directory list applies; `classify_test_file`, testfile.rs:50-160).
- `TEST_CLASS_MEMBERS_ARE_ROOTS` is python/php/ruby only (rules.rs:11806).
- `std.testing.expect` inside a test block → stdlib bucket (fixture p1 line 71).

## 12. Dead-code analysis

Fixture p1 `--dead-code`: signal table row `zig … class degraded, vis no, attrs no, refs no, disp no`; `findings 0 shown (high) · 10 withheld (medium)`.

- **Class `Degraded`**: the plugin declares no visibility or attributes (`signals()` default) and none are observed (caps.rs:115-176), so every Zig finding carries `language-lacks-visibility`, `language-lacks-attributes`, `language-lacks-value-references`, `language-lacks-dispatch-model` and is capped at **medium** (never high). Fixture p1 findings: `build`, `c_entry`, `callc`, `puts`, `inl`, `unusedPrivate`, `unusedPublic`, `name`, `testHelperName`, `other`.
- **Roots found**: `main` (`builtin:main`, `UNIVERSAL_ENTRY`, roots.rs:20), `_start` (`LANG_ENTRY` zig, roots.rs:29), test-name roots (§11), top-level invocations from `test`/`comptime` blocks (§11).
- **Not roots**: `pub` functions (no visibility recorded — `unusedPublic` reported), `export fn` / `callconv(.c)` (no `ffi:export`; `c_entry`, `callc` reported), `@export`ed functions (rooted only incidentally if called from top level), `build` in `build.zig`, `extern fn` prototypes (`puts` reported as dead).
- **Value uses invisible**: a function only passed as a value (`callback(local)`, `@call(.auto, f, …)`, vtable initialisers) has no incoming edge (`language-lacks-value-references`).
- **Unresolved-doubt**: a same-named unresolved site should downgrade a finding; fixture p1 `name` still shows `no-unresolved-site-with-name` although `Color.red.name()` was recorded (in the external bucket), so external-bucket sites do not count as doubt for it.
- No caps from `implements-external-contract`, `macro-generated`, `reflection` apply (no such signals for Zig).

## 13. Builtins & stdlib tables

- `crates/cgg-core/src/stdlib/zig.txt`: 160 names (`alloc`, `ArrayList`, `assert`, `print`, `std`, `testing`, `Writer`, …), embedded at stdlib.rs:204, registered at :245.
- Used by the external classifier (`external.rs:186-260`): a receiver headed by a stdlib name (`std.debug`, `std`) or a bare stdlib name the tree does not define goes to the stdlib bucket; fixture p1 `std.debug.print` and `assert` → stdlib; `build.zig` `b.addModule`/`b.path`/… → stdlib (receiver typed `std.Build` through the parameter). Also excludes stdlib names from the step-1d global by-name fallback (cross_file.rs:6018-6024).
- `--include-stdlib` mints `<stdlib>::…` exit nodes at low (fixture p1x: `<stdlib>::std.debug::print`, `<stdlib>::assert`, `<stdlib>::std.Build::addModule`; p2 `<stdlib>::std.ArrayList(u8)::init`). `--include-external` mints `<external>::…` (p1x `<external>::ext::doThing`).
- When the tree *contains* `std` (Zig compiler repo shape, fixture p3), calls resolve into it rather than to the stdlib bucket.
- No Zig rows in `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `MAYBE_TYPE_WRAPPERS`, `NULL_TYPE_MEMBERS`, `TOP_TYPES`, `CALLBACK_TYPE_CONSTRUCTORS`, implicit-package lists (stdlib.rs; grep finds no `zig` outside the manifest lines). No `rust_types`-like wrapper table.

## 14. Confidence & certainty

- **High**: intra-file bare call with one same-file candidate (fixture p1 `local`, `cleanup`); `resolve_zig` module path to one function (`certain: true`, libscope.rs:1479-1486; p1 `lib.libEntry`, `mylib.libEntry`; p3 `std.debug.print`); container/typed-receiver member with one callable (libscope.rs:1561-1567; p1 `c.inc()`, p3 `x.inc()`); plain function's declared-return-type receiver (libscope.rs:1720-1725; p3 `types.makeCounter()` then `k.inc()`); file-level alias followed to the tree module (`*module_certain = true`, cross_file.rs:5807-5811; p1 `helperAlias`). `module_certain` is set only when exactly one callable is bound (cross_file.rs:5461-5463).
- **Medium**: type-function result members (`certain: false`, libscope.rs:1765; p1 `lib.List(u8).init()`, `l.push`), container-level alias via a type function (p3 `w.frame()`), re-export alias called bare (p1 `lib.zig:6 helper(x)`), duck fan-out on untyped receivers (p3 `y.inc()`, payloads, field chains; p2 `self.*.visit2()`).
- **Low**: only synthetic exit-node edges (`--include-external`/`--include-stdlib`).
- Mermaid prints `?` on arrows with no high site: fixture p3 `N3 -->|?| N9`, `N4 -->|?| N7` (duck edges).
- `weak_receiver_type` is never set for Zig sites (plugin does not set it; type_hints sets it only from the `weak` set it builds — unverified for Zig specifics).

## 15. Dynamic use & reflection

- `dynuse` covers Python, JS, Java, C#, Ruby and PHP only (CLAUDE.md; `crates/cgg-lang/src/plugins/dynuse.rs` has no Zig entry). Zig declares no `dyn_uses` signal.
- `@field(obj, "name")`, `@hasDecl(T, "name")`, `@call(.auto, f, …)`, `std.meta` iteration, `@export(&f, …)`: not recognised as reflection or value use; a call whose callee is `@field(...)` becomes an external site with the builtin text as its name (fixture p1 line 42).
- Module loads are static `@import` only; no dynamic loader concept.

## 16. Known gaps & limitations

- **`test` blocks are not callables** (rules.rs:6608 gap text; fixture p1). Calls in them are rooted as production top-level invocations.
- **No visibility / no attributes** (`pub`, `export`, `extern`, `inline`, `callconv` dropped; zig.rs:200-201; rules.rs:6625). Consequences: dead-code class `Degraded`, every finding capped at medium, `pub`/`export` functions reported as dead.
- **Qualified names have no owner** (zig.rs:192-193): same-named members of several containers in one file are indistinguishable in output and bare calls to them are `ambiguous-in-file` (fixture p4). No lexical narrowing for Zig (intra_file.rs:119-135).
- **Field-chain receivers** (`self.child.m()`, `self.vtable.write()`, `priv.config orelse …`) are untyped — CHANGELOG.md:705-706 ("Not covered (WS14b)"); CHANGELOG.md:1394-1396 notes Zig recovered only 105 of 437 `candidates-in-other-files` and 86 of 266 `shadowed-by-local` sites ("the rest: field-chain receivers, comptime function parameters").
- **Payload captures and `anytype`/`comptime` parameters untyped** → duck fan-out (fixture p3).
- **Containers declared inside a function** cannot be named (`Local.inner()` external, fixture p2/p4; zig.rs:342-345).
- **No value references** (functions passed as values, `@call`, vtables) — `signals()` default.
- **No FFI linking** of `export fn` / `extern fn` with C (fixture p5); `extern fn` prototypes become callables that absorb same-file calls at high.
- **`usingnamespace`**, `@cImport`, `build.zig.zon` dependencies, `b.createModule`/`addImport` not followed.
- **Error recovery can drop definitions** (fixture `p2/err.zig`).
- **Duplicate bucket records**: a site bound later by cross-file resolution can also appear in the external bucket and get an `<external>` exit node (fixture p1 line 24 `helperAlias`: high edge to `util.helper` *and* `<external>::helperAlias`; p2 line 8 `write` in both unresolved and external buckets). Observed, cause not investigated.
- **README stale**: README.md:506 lists Zig type inference as "—" although WS14b added local/parameter typing.
- **Performance note**: `zig-zig` was ~12% slower with audit due to Zig library scopes (CHANGELOG.md:1167-1170); `zig-zig` exceeds 60 s under `--dead-code` (CHANGELOG.md:1583).
- No ignored tests mention Zig (grep `#[ignore]` with zig finds none — unverified beyond grep of test names listed above). Tests relevant to Zig: zig.rs `functions_extracted`, `a_member_alias_is_recorded`, `calls_and_imports`, `import_names_and_build_modules`, `locals_are_typed_from_returns_and_annotations`; locals.rs `a_zig_same_named_member_alias_is_not_a_binding` + multi-language table; libscope.rs `zig_return_types_are_peeled`; cross_file.rs `module_alias_copies_need_evidence`, `zig_member_aliases_and_own_declarations`, `zig_alias_of_a_dependency_is_external_and_of_the_tree_is_followed`, `zig_import_paths_and_build_modules`, `zig_container_typed_receiver_reaches_its_member`; `crates/cgg/tests/detect_prefixes.rs` (zig arm). Not run for this document.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.zig` only; no shebang, no `.zon`; no re-parse; depth/minified handling generic |
| 02. Callables extracted | partial | all `fn`s incl. container members, but qn has no owner, no `test` blocks, no visibility/attrs |
| 03. Call-site forms | partial | bare, module-path, container, typed, generic-result calls; no value refs, field chains, builtins, arity |
| 04. Imports & module resolution | full | `@import` files, build.zig modules, std-style roots, aliases, re-exports, external refusal; no usingnamespace/zon |
| 05. Type inference & receiver typing | partial | params, written/literal/call-result locals, container spans, `@This()`; no fields, payloads, casts |
| 06. Inheritance, dispatch & overloading | n/a | no classes/overloads in Zig; vtable pattern unmodelled |
| 07. Scoping & shadowing | partial | locals/params/payloads shadow positionally; no lexical narrowing of container members |
| 08. Macros, preprocessor & codegen | minimal | type-function `return struct` members bind at medium; no comptime reflection |
| 09. FFI & cross-language links | none | `export fn` / `extern fn` / `@export` never linked to C |
| 10. Frameworks & entry points | minimal | 4 detect-only rules (zig-main/build/test/export), none mint nodes |
| 11. Test harness entries | none | test blocks not callables; zig-test detect-only; only `test_*`/`Test*` name roots |
| 12. Dead-code analysis | minimal | Degraded class, all findings medium; only `main`/`_start`/name/top-level roots |
| 13. Builtins & stdlib tables | partial | 160-name `zig.txt`; no cast/constructor/wrapper tables |
| 14. Confidence & certainty | full | high for exact module/container/typed paths; medium for generic results and duck fan-out |
| 15. Dynamic use & reflection | none | `@field`/`@call`/`@hasDecl` not recognised |
| 16. Known gaps & limitations | n/a | see list: test blocks, visibility, owner-less qns, field chains, FFI |
