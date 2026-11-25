# go

> Revised 2026-11-25 against cgg 0.13.3: Go now has a locals table, name-first parameter and receiver typing, call-result typing, value references outside registrars, embedded-field and embedded-interface bases, `abstract` interface specs, interface method-set dispatch (`go_iface`) and consumed unreachable regions; the sections below are corrected accordingly. `file:line` citations not otherwise revised date from the 0.11.0 audit and may have drifted.

The Go plugin (`GoPlugin`, `crates/cgg-lang/src/plugins/go.rs`, 1,796 lines including inline tests) parses `.go` files with the crates.io grammar `tree-sitter-go` (`tree_sitter_go::LANGUAGE`, go.rs:50-52). Nothing is vendored. A single recursive `Walker` (go.rs:135-923) does the following:

- Records free functions, methods (owner = receiver type), interface method specs, `:=`-bound func literals and synthetic `handler_at_<line>` closures for inline route handlers.
- Records call sites: bare, selector, explicit generic instantiation, and receiver shapes peeled by `crate::receiver::peel`.
- Records imports, plus three synthetic import records:
  - `package-root`: the `package` clause.
  - `package-path`: the file's import path under `go.mod`.
  - `go.mod` module paths into `FileFacts::packages`.
- Records a small set of local types, and records embedded struct fields and embedded interfaces as `ClassDecl` bases (go.rs:708-764).
- Records function and method values in value positions as `value_ref` sites (go.rs:363-470) when reference sites are on.
- Parameters, receivers and locals are collected by the shared locals table (`GO_SPEC`, locals.rs:391).

The plugin declares the signals `test_defs`, `unreachable`, `value_refs` and `visibility` (go.rs:40-48). It does **not** declare `attributes`, `exports`, `impls` or `dyn_uses`, even though it does extract `dyn_uses` (go.rs:114-116). No `resolver_kind` override is declared; resolution uses the generic pipeline. Go-specific branches exist in:

- `cgg-resolve/src/modules.rs`: `ModuleIndex`/`FileModules`, `resolve_go`.
- `heads.rs`: `go_workspace`, `go_package_name`, external-module refusal.
- `cross_file.rs`: step 0c, the bare-call reach rule, and `unique_owner_names` keyed per directory.
- `type_hints.rs`: `go_package_param_types` and scoped package-made locals.
- `intra_file.rs`: `bare_call_is_never_a_method`.
- `type_hints.rs`: `go_param_types` (type_hints.rs:2211), the name-first reader for receivers and parameters.
- `modules.rs`: `go_call_result` (modules.rs:2211), the declared result of a same-package call.
- `go_iface.rs`: Go method sets, `go_interface_fanout` and `go_interface_site_edges` (§06).

Fixtures cited below live under `<scratch>/parity/go/` (f1 … f15). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, and dumped with `show.py` in the same directory. Unless a fixture says otherwise, it has a `go.mod` (module mode).

## 01. Detection & parsing

- **Extension `.go` only.** Evidence: go.rs:37-39; detect.rs:685 test table (`("g.go", "go")`). Fixture f1: `detected_via: "extension:.go"`.
- **Shebangs**: none. The plugin does not override `shebangs()` (default `&[]`, cgg-lang lib.rs). A `//usr/bin/env go run` header is not recognised.
- **Content detection**: none for Go.
- **`go.mod` is read, but not as a source file.** The walker skips it as `unknown-extension` (fixture f1 audit `file_skipped ./go.mod`). The plugin reads it during extraction through `manifest::go_module_info` (manifest.rs:29-63): it walks up to the nearest `go.mod` and reads:
  - the `module` directive;
  - local `replace … => ./dir` directives (`parse_go_mod`, manifest.rs:152-187; `local_replace` manifest.rs:189+).
- **Grammar**: a single grammar. No dialects, so there is no `ts_language_for` override.
- **Re-parse/fallback**: none. Unlike C#, there is no `#if` blanking. A file with syntax errors is extracted with `parse_status = "error"`, as for every language (cgg/src/lib.rs).
- **Depth skip**: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep`. This is shared by every language (cgg/src/lib.rs, `cgg_lang::exceeds_depth`).
- **Minified handling**: Go now has a locals spec (`GO_SPEC`), so the generic "no locals on a line over 2,000 bytes" rule applies. `max_line_bytes` is not overridden (default `None`).
- **Package name fallback**: when there is no `package` clause, the package name is the file stem (go.rs:65-71).
- **Generated code** (`*.pb.go`, `// Code generated … DO NOT EDIT.`) gets no special handling. A grep over the crates for `pb.go|DO NOT EDIT|Code generated` finds only rule gap prose. Such files are parsed like any other.
- **`vendor/` is skipped** by the walker's built-in deny list (cgg-walk/src/lib.rs:54). Vendored packages are therefore never analyzed. An import of one is judged by `go.mod`/directory rules (§04).
- **Build constraints** (`//go:build linux`) are not read. Both platform twins are parsed and both are extracted. Fixture f12: `p_linux.go` and `p_windows.go` each define `plat`, and `Use() { plat() }` binds **both** at medium.

## 02. Callables extracted

A qualified name is `<package clause>.<name>` for functions and `<package clause>.<ReceiverType>.<name>` for methods (go.rs:293, 340-344). There is **no directory or import path** in the QN. Two packages both named `util` in different directories produce identical QNs. They are told apart by file and directory in `go_lookup` (modules.rs:1358-1374) and `unique_owner_names` (cross_file.rs:4415).

Supported shapes:

- **Function declaration** `func Plain() {}` → `FreeFunction`, QN `pkg.Plain` (record_function, go.rs:285-311). Test `free_function` (go.rs:1117). Fixture f1 `jobs.NewRunner`.
- **Generic function** `func Generic[T any](x T) T` → `FreeFunction` `jobs.Generic`. Type parameters are not part of the name. Fixture f1.
- **Method with a value or pointer receiver**, e.g. `func (r *Runner) Run()` or `func (r Runner) step()`, → `InherentMethod` `jobs.Runner.Run`. Pointer and value receivers are treated the same (go.rs:9-11, 461-474). Test `method_qualified_name_has_receiver_type` (go.rs:1127). Fixture f1.
- **Method on a generic type** `func (m *Map[K, V]) Put(…)` → `jobs.Map.Put`; the type arguments are dropped (`receiver_type_name` `generic_type` arm, go.rs:469-471). Test `generic_receiver_keeps_its_type` (go.rs:987). Fixture f1.
- **Method with a parenthesized receiver type** `func (r (*T)) M()` is handled by the `parenthesized_type` arm (go.rs:464-468).
- **Receiver with no name** `func (Foo) Quux()` → `z.Foo.Quux`. Fixture f15.
- **Method on a function type** `type HandlerFunc func(int); func (f HandlerFunc) Serve(x int)` → `w.HandlerFunc.Serve`. Fixture f13.
- **Interface method specs** `type Doer interface { Do() error }` → `InherentMethod` `jobs.Doer.Do`, with `signature_hint` = the spec text (extract_interface_methods, go.rs:405-456). Fixture f1 `jobs.Doer.Do`, `jobs.Doer.Name`.
  - Since WS22 they are tagged `abstract` (test `interface_method_specs_are_tagged_abstract`, go.rs:1683), so duck-typed fan-out drops them when a concrete method survives. 0.13.3 fixture `lang-docs/go/f1`: `lib.Shape.Area` carries `abstract`, and `iface(s lib.Shape) { s.Area() }` binds `lib.Shape.Area` at high (typed parameter).
- **Func literal bound with `:=`** `add := func(a int) int {…}` → `FreeFunction` named after the variable, QN `pkg.add` (go.rs:576-606). Calls inside it are attributed to it. Fixture f1 `main.add`. Fixture f9: `inner := func() { shared() }` gives node `a.inner` with edge `a.inner -> a.shared`.
- **Inline route handler** `r.POST("/inline", func(c *gin.Context) {…})` → `NamedClosure` `pkg.handler_at_<line>` with attribute `synthetic`, plus a value-ref from the call (extract_inline_handler, go.rs:234-271). It is gated on a registrar verb and on the registration shape. Test `inline_route_handler_gets_a_node` (go.rs:1096). Fixture f5: `svc.handler_at_32` with edge `handler_at_32 -> svc.unexported`.
- **Visibility**: `Vis::Public` when the first character is uppercase, otherwise `Vis::Internal` (go_vis, go.rs:940-945). This applies to functions, methods, interface members and `:=` literals.
- **Test role**: `TestMain` → `TestRole::Fixture`. A name of the form `Test|Benchmark|Fuzz|Example` + an uppercase letter → `TestRole::Case` (go_test_role, go.rs:950-963).

Not supported or partial:

- **Package-level `var F = func(){…}`** is not extracted; only `short_var_declaration` mints a literal node. Fixture f1: `var Hook = func(){}` has no node, and `jobs.Hook()` is unresolved (audit external bucket).
- **Local `var v = func(){}`** inside a function is not extracted. Fixture f13: there is no node `v`.
- **Multi-assigned literals** `a, b := func(){}, func(){}` mint **one bogus node named `a, b`**. The var name is the left side's full text (go.rs:517, 585). Fixture f13: node `w.a, b`.
- **Anonymous func literals** passed to non-registrar calls (`go func(){}()`, `defer func(){}()`, `sort.Slice(x, func…)`, `t.Run("x", func…)`) get **no node**. Their calls are attributed to the enclosing function (go.rs:163-167 only records parameter shadows). This is also disclosed in the `go-runtime` rule gap (rules.rs:7702). Fixture f1: `defer func() { local() }()` gives the edge `main.main -> main.local`.
- **Nested `:=` literals get package-level QNs**. `inner := func(){}` inside `outer` is `pkg.inner`, not `pkg.outer.inner`. A local literal named like a package function produces two nodes with the same QN. Fixture f10: two `t.handler` nodes.
  - Since WS27 a call through the local binds the closure its own declaration defines: 0.13.3 fixture `lang-docs/go/f3` has two `a.check` nodes (`t1`, `t2` each declare `check := func(){}`), and each `check()` binds only its own at high.
- **Embedded interfaces** (`type ReadWriter interface { Reader; Write(…) }`) record only the explicit `Write` as a callable; since WS23 the embedded `Reader` is recorded as a `ClassDecl` base (go.rs:708-764). 0.13.3 fixture `lang-docs/go/f2`: `r.Fetch()` on `r lib.RW` (RW embeds `Reader`) binds `lib.Reader.Fetch` at medium.
- **Struct types, fields and type declarations** are not callables. Struct embedding is recorded as `ClassDecl` bases (same code), so promoted methods resolve through the base chain (§06).
- **Overloads/clauses**: not applicable (Go has no overloading).
- **Operators**: Go has no operator overloading. `x.Add(y)` is an ordinary method (fixture f13 `w.Vec.Add`).

## 03. Call-site forms

All are recorded by `ref_from_call` (go.rs:809-847) and `ref_from_generic_conversion` (go.rs:855-887).

- **Bare call** `local()` → `name=local`, no receiver. Fixture f1 `main.main -> main.local` (high, intra-file).
- **Selector call** `fmt.Println(…)`, `r.Run()`, `s.r.Run()` → `name` = the field; `receiver_hint` = the operand text as written (go.rs:814-828). Test `call_expressions_captured` (go.rs:1168).
- **Peeled receiver shapes** (`crate::receiver::peel("go", …)`, receiver.rs:86-101, 170-178):
  - `(f).M()` → `f`.
  - `Foo{A: 1}.M()` / `(&Foo{}).M()` → typed `Foo`.
  - `o.(*pkg.Foo).M()` → typed `pkg.Foo`.
  - `(*p).M()` → `p` (deref).
  - These set `RefRecord::typed_receiver` for the literal and assertion forms. Test `go_literals_assertions_and_derefs` (receiver.rs:485-500); integration test `shaped_go_receivers_name_their_type` (crates/cgg/tests/resolve.rs:3155). Fixture f1: `(&Server{}).Handle()`, `Server{}.helper()` and `i.(*Server).Handle()` are all high.
- **Chained call through a package function's result** `jobs.NewRunner().Run()` → receiver `jobs.NewRunner()`, resolved by the callee's declared result type (`go_result_type`, modules.rs). Fixture f1: high to `jobs.Runner.Run`.
  - Since WS27 a same-package chain `build().Handle()` resolves through `build`'s declared result (`go_call_result`, modules.rs:2211), when some Go function of that name declares one. 0.13.3 fixture `lang-docs/go/f1`: `build().Handle()` binds `a.Server.Handle` at high.
- **Method expression** `(*l2.Derived).Own(d)` → high to `lib.Derived.Own` (fixture f6).
- **Explicit generic instantiation**:
  - `jobs.Generic[int](3)` (parsed as `type_conversion_expression`) and `jobs.Pair[int, string](1, "a")` (an `index_expression` callee) are recorded under the bare function name. A selector head qualifies only when it is an imported package (go.rs:184-190, 832, 855-923).
  - Tests `explicit_instantiation_calls_are_recorded` (go.rs:1204) and `indexed_function_values_are_not_instantiations` (go.rs:1227).
  - Fixture f1: both are high.
- **Type conversions** `int(x)`, `MyType(x)` are recorded as calls named after the type. They land in the stdlib bucket, or find no callable (fixture f7: `int` in `stdlib_calls`).
- **`go f()` / `defer f()`** are ordinary call expressions. Fixture f6: `go Worker()` gives the edge `app.Run -> lib.Worker`.
- **Registrar arguments**: on a call whose last callee segment is a Go registrar verb (`ctx.is_registrar_verb`, language-scoped: lib.rs:395-406 test `the_registrar_gate_is_scoped_to_one_language`), `registrar::capture` (registrar.rs:424-500) records:
  - identifier arguments as value refs (`VALUE_REF_HINT`);
  - later string arguments as string refs;
  - the route.
  - Fixture f5: `r.GET("/users", listUsers)` gives `Wire -> listUsers [medium reference]` plus the gin entry.
- **Arity**: not recorded. The Go plugin never sets `RefRecord::arity`, and Go is not in `OVERLOAD_ARITY_LANGS`.
- **Keyword args / paren-less / implicit receiver / invocant**: not applicable, never set.

- **Function and method values** (WS26): under `--reference-edges` or `--dead-code`, a function or method value in an argument outside registrar verbs, a composite-literal value (struct field values, map and slice elements), an `=`/`:=`/`var` right-hand side or a `return` is a `value_ref` site, bare, through the package or through a variable (go.rs:363-470). A name a parameter or local in scope binds is pruned through the locals table. A method value through an untyped variable never fans out by name. 0.13.3 fixtures `lang-docs/go/f1` and `f3`: `sortSlice(nil, less)` gives `UseLess -> lib.less [high reference]`, and `map[string]func(){"k": mk}` gives `vals -> a.mk [high reference]`.
- **Member-read aliases**: `f := s.Load; f()` is rewritten to `s.Load()` (`cgg_lang::member_aliases`). 0.13.3 fixture `lang-docs/go/f3`: with `s *S`, `f()` binds `a.S.Load` at high.

Not supported:

- **Package-level `var F = f` aliases** are not followed: in fixture `lang-docs/go/f3`, `var New = mk` at top level is a value reference with no enclosing callable (`no-enclosing-callable`), and `New()` binds nothing.
- **Callee shapes other than identifier, selector or index** are dropped: `func(){}()`, `getF()()`, `(f)()`, `fns[i]()` with a non-type index (go.rs:833 `_ => return None`; go.rs:897-911).
- **Calls through struct fields of func type** (`s.cb()`) are read as method calls named `cb` (from the selector shape). They never bind to the function stored in the field.

## 04. Imports & module resolution

Plugin side:

- **Import forms**: `import "fmt"`, grouped `import (…)`, aliased `al "path"`, dot `. "path"`, blank `_ "path"`. Each spec becomes `ImportRecord{kind:"import", path, alias}` (go.rs:364-403). Test `imports_captured` (go.rs:1146).
- **Synthetic records**:
  - `package-root` (path = package clause; go.rs:75-81; test `package_root_marker_emitted` go.rs:1187).
  - `package-path` (alias = `module` + relative directory; go.rs:96-109, manifest.rs:37-57).
- **`FileFacts::packages`** = the nearest `go.mod`'s `module` plus every locally `replace`d module (go.rs:86-87, manifest.rs:18-28).

Resolver side:

- **Workspace vocab** (heads.rs:117-125): every Go file's `packages` plus every directory suffix of every Go file's directory (`go_dir_suffixes`).
- **`go_workspace(path, file_has_module)`** (heads.rs:270-293): an import path is the workspace's when it matches one of these:
  - it equals or lies under a `go.mod` module;
  - it equals a directory suffix;
  - in GOPATH mode (no `go.mod`), its last segment names a tree directory.
- **Package name implied by a path** (`go_package_name`, heads.rs:1010-1027): drops a `/vN` or `.vN` major version and a `go-` prefix. Test `go_package_names` (heads.rs:1097): `github.com/go-redis/redis/v8` → `redis`, `gopkg.in/yaml.v3` → `yaml`. An unaliased import binds both the raw last segment and the implied name (heads.rs:487-505; `go_import_names`, modules.rs).
- **External-module refusal** (heads.rs:665): a receiver headed by a name bound to an external import is recorded as `external-module` and never name-matched.
  - Fixture f1: `uuid.New()` (`github.com/google/uuid`) goes to the external bucket.
  - Fixture f8: `pkg.Do2()` gives `external-module github.com/ext/pkg`, although a workspace `Do2` exists.
  - Tests `go_call_into_a_module_outside_go_mod_is_external` (crates/cgg/tests/resolve.rs:2690), `go_import_outside_every_module_is_external`, `go_stdlib_import_named_like_a_workspace_directory_is_external` (cross_file.rs tests).
- **Module-scoped lookup, step 0c** (cross_file.rs:5388-5420 → `FileModules::resolve` → `resolve_go`, modules.rs:1987-2084):
  - **Bare call**: looked up in the caller's own package. These are files in the same directory with the same `package` clause (`go_same_package`, modules.rs:1335-1354) and the result is `certain:true`. Fixture f9: `three -> a.shared` (in `a2.go`) is high. Test `go_bare_call_binds_in_its_own_package_first`, `go_bare_call_stays_in_its_package`.
  - **`pkg.F()`** with the package itself as receiver: looked up in that package's files. A miss is a refusal, `not-in-imported-module`, recorded only when some callable carries the name (cross_file.rs:2810-2827).
    - Fixture f6: `l2.Run()` gives `not-in-imported-module ex.com/x/lib`.
    - Fixture f1: `jobs.Missing()` names nothing anywhere and is only bucketed (audit `external_calls`, screen `external`).
  - **`pkg.T` typed receiver**: `T.method`, else through `bases(...)` (certain:false).
  - **`pkg.NewX().M()`**: through `go_result_type` of the callee, following another package via the callee file's imports (`go_imports`).
  - **Package files** (`go_files`, modules.rs:1246-1290): found by exact `package-path` first, else by a unique directory-suffix match. Several matching directories yield nothing. **Files whose package clause ends in `_test` are excluded.** Fixture f8: `util_test.Do2` is not chosen for `util.Do2()`.
  - Test `go_import_path_decides_between_same_named_packages`.
- **Module mode**: an import path no `go.mod` of the tree covers and no `package-path` names is external, even if a directory shares its name (modules.rs:2014-2022).
- **GOPATH mode** (no `go.mod`): the import is resolved by directory name (fixture f8: `src/cmd/m.go` `util.Do2()` and `weird.W()` both high, the latter with package clause `notweird`).
- **Package clause ≠ directory**: handled; the lookup uses the file's `package-root` (fixture f8 `weird.W()` → `notweird.W`).
- **Aliased import** `j2 "…/jobs"; j2.SpawnJob()` → high (fixture f1). Test `go_aliased_import_resolves` (resolve.rs:360).
- **Dot import** `. "ex.com/x/lib"; Helper()` → **medium** by-name. The Go bare-call reach rule allows any directory when the file has a `.` alias. Fixture f6.
- **Blank import** `_ "embed"` binds nothing (heads.rs:493-498, modules.rs `go_import_names`).
- **Re-exports**: not applicable. Go has none; type aliases (`type A = b.T`) are not followed.
- **Not covered: package-variable function aliases**. `var New = sdk.New` is called as `pkg.New()`, defines no callable, and is refused or bucketed (CHANGELOG.md:1320-1321).
- **Partial: an external dot import leaks.** `import . "github.com/onsi/ginkgo"; Describe()` binds to a workspace `b.Describe` in another directory at medium (fixture f14; still so in 0.13.3 fixture `lang-docs/go/f2`). The dot alias opens the any-directory rule without checking that the dot-imported package is the workspace's.
- **cgo `import "C"`**: `C` is treated as an external module (fixture f7: `C.add` gives `external_calls`, receiver `C`).

## 05. Type inference & receiver typing

Plugin-recorded local types (`FileFacts::local_types`):

- **`var x T`** (single spec, single name, uppercase unqualified type, leading `*` stripped) → `x: T` (record_var_type, go.rs:480-508). Fixture f4: `var v Server; v.Zap()` is high.
- **`x := NewFoo()` / `newFoo()`** → `x: Foo`. This is a naming-convention heuristic on the callee name (go.rs:608-639). Fixture f4: `n := NewServer(); n.Zap()` is high. Fixture f9: `x := NewB()` is high.
- **Package-made values** (go.rs:521-551, 734-774) are typed as written:
  - `r := jobs.NewRunner()` → `jobs.NewRunner()`;
  - `r, err := jobs.Open(p)` → `jobs.Open()` (first name only);
  - `w := &jobs.Runner{}` / `jobs.Runner{}` → `jobs.Runner`.
  - Each is in scope from the end of the statement to the end of the enclosing block (an untyped closing entry).
  - Tests `package_made_local_starts_after_its_statement` (go.rs:1041), `values_made_through_a_package_are_typed_as_written` (go.rs:1058), `go_package_made_locals_respect_declaration_and_block` and `go_package_made_locals_are_scoped_to_their_function` (type_hints.rs tests).
  - Fixture f1: `r.Run()`, `o.Do()` and `w.Run()` are high.
- **Package-qualified parameters** `func f(r *jobs.Runner, d jobs.Doer)` → `r: jobs.Runner`, `d: jobs.Doer`. Grouped names `a, b T` take the next type (`go_package_param_types`, type_hints.rs:1489-1530). Fixture f1: `useParam` gives `r.Run()` → `jobs.Runner.Run` (high) and `d.Do()` → `jobs.Doer.Do` (high).
- **Typed receiver shapes** from peel (§03): composite literals and type assertions.
- **Receivers and same-package parameters** (WS23): `go_param_types` (type_hints.rs:2211) reads the receiver and every parameter name-first, skipping predeclared types (`GO_PREDECLARED_TYPES`), type parameters and composite types. A bare type binds within the caller's package (directory). A `:=` in an inner block, an `if`/`for`/`switch` header or a func literal's own parameter hides the parameter there. 0.13.3 fixture `lang-docs/go/f1`: `func viaParam(q *Server) { q.Zap() }` binds `a.Server.Zap` at high, and `func (s *Server) Zap() { s.zip() }` binds `a.Server.zip` at high. Func-literal parameters are typed the same way (fixture `lang-docs/go/f2`: `f := func(a A) { a.Zork() }` binds `a.A.Zork` at high).
- **Same-package call results** (WS27 follow-up): `x := f(…)`, `x, err := f(…)` and `f(…).M()` take the result type `f` declares (`go_call_result`, modules.rs:2211), only when some Go function `f` declares one, and never for builtins, a parameter or local called as a function value, or a single-name `v := NewFoo()`, which keeps its `Foo` reading. 0.13.3 fixture `lang-docs/go/f1`: `x := build(); x.Zap()` binds `a.Server.Zap` at high.

Resolver handling (`propagate_types_with_returns`, type_hints.rs:405-483, 772-783):

- Go's own readings (`var T`, `New*`) are **file-wide, last write wins** (`local_type_map`).
- Only package-made values are looked up where they are in scope (`RustLets::visible` + `go_package_made`).

Not supported or partial:

- **Same-package composite literals and `new(T)` in `:=`**: `q := &Server{}`, `t := Server{}` and `z := new(Server)` are untyped (fixture f2, f4; still so in 0.13.3 fixture `lang-docs/go/f1`, where `z.Zap()` fans out to `Server.Zap` and `Other.Zap` at medium). `package_made_type` requires a `qualified_type`.
- **Package-qualified `var`**: `var x jobs.Runner` is not typed, because the uppercase-first check fails on `jobs.`. 0.13.3 fixture `lang-docs/go/f2`: `var x lib.Runner; x.Zork()` fans out to three `Zork` methods at medium.
- **Grouped `var ( a Foo )`** is not typed, and **`var p, q Foo`** types only the first name (fixture f15; 0.13.3 fixture `lang-docs/go/f2`: `p.Zork()` high, `q.Zork()` fans out).
- **File-wide last-write-wins still gives a wrong edge, now at medium**. In fixture f9, `one(){ var x A; x.Zork() }` bound to `B.Zork` at high. In 0.13.3 fixture `lang-docs/go/f1`, `one(){ var x Server; x.Zap() }` binds only `a.Other.Zap` at medium, because `two` declares `var x Other` later in the file.
- **Struct fields** are not typed: `s.r.Run()` and `h.srv.Handle()` are unresolved (fixture f1, f3; still so in 0.13.3 fixture `lang-docs/go/f1`, `candidates-in-other-files`). The plugin records no `class_fields`.
- **Type switches / `x, ok := y.(T)`** bindings are not typed (they are locals, §07). Only the inline `y.(T).M()` shape is read. 0.13.3 fixture `lang-docs/go/f2`: `switch t := v.(type) { case A: t.Zork() }` and `if t, ok := v.(B); ok { t.Zork() }` both fan out at medium.
- **Range variables and channel receives**: untyped. 0.13.3 fixture `lang-docs/go/f2`: `for _, x := range xs { x.Zork() }` with `xs []A` fans out at medium.
- **`declared_type` tables**: no Go entries in `MAYBE_TYPE_WRAPPERS`, `UNION_TYPE_CONSTRUCTORS`, `NULL_TYPE_MEMBERS`, `TOP_TYPES` or `CALLBACK_TYPE_CONSTRUCTORS` (stdlib.rs:104-160). `any`/`interface{}` are not treated as top types.
- **Weak receiver types**: never set. The `New*` naming guess is treated as a firm type and yields `high` (fixture f4).
- **Implicit receivers**: not applicable.

## 06. Inheritance, dispatch & overloading

- **Base records** (WS23): embedded struct fields and embedded interfaces are recorded as `ClassDecl` bases (go.rs:708-764). The plugin header comment (go.rs:19-21) still says embedding is not modeled; it is stale.
- **Promoted methods via embedding**: `d := &l2.Derived{}; d.Hello()` (`Hello` on embedded `Base`) now reaches `Base.Hello` through the base chain. 0.13.3 fixture `lang-docs/go/f1`: `promo -> lib.Base.Hello` at medium (a base-chain lookup is never `high`).
- **Interface → implementation** (WS31, `cgg_resolve::go_iface`): Go is still **not** in `INHERITANCE_LANGS`; instead `go_iface` reads Go method sets (an interface's declared methods plus those of embedded interfaces; a named type's value- and pointer-receiver methods plus those promoted through embedded fields), keyed by package directory, with qualified embeds found through the file's imports and standard-library interfaces in `cgg_core::stdlib::GO_STDLIB_INTERFACES` (stdlib.rs:573).
  - Under `--dynamic-dispatch` (and `--dead-code`), `go_interface_fanout` (go_iface.rs:381) adds a `low` `dyn` edge, resolver `dispatch:go-interface`, from each interface method declaration to the same-named method of every satisfying type (by name and, where both signatures show it, parameter count). 0.13.3 fixture `lang-docs/go/f1`: `lib.Shape.Area -> lib.Sq.Area` and `-> lib.Circ.Area`; fixture `lang-docs/go/f2`: `RW.Store -> T.Store` and the embedded `Reader.Fetch -> T.Fetch`.
  - An interface that only repeats a standard-library one fans out to nothing (in fixture `lang-docs/go/f2`, a local `Reader { Read() }` got no `dyn` edges until renamed).
  - Under `--dead-code` only, `go_interface_site_edges` (go_iface.rs:424) links an unresolved method call (cap-exceeded or candidates elsewhere) to the declarations of that method in in-tree interfaces of the calling package or a package it imports.
  - The default graph is unchanged: a call through a typed interface binds to the interface declaration only (fixture f1: `d.Do()` → `jobs.Doer.Do`, high).
- **super/base calls, `member_of_calling_class`, `overrides_below`**: not applicable. Go has no inheritance, and a bare call never reaches a method (`bare_call_is_never_a_method` includes `"go"`, intra_file.rs:49-54; cross_file.rs:4154-4158 `!member`).
- **Extension methods / mixins / traits**: not applicable.
- **Overloading / arity narrowing**: not applicable; Go is not in `OVERLOAD_ARITY_LANGS`.
- **Generic specialization narrowing** (`narrow_specializations`): not exercised. Go owners are recorded without type arguments.

## 07. Scoping & shadowing

- **`FileFacts::locals`** (WS27): Go has a locals spec (`GO_SPEC`, locals.rs:391). Parameters, receivers, named results, `:=`, `var`/`const`, `for`/`range`, `if`/`switch` initializers, type-switch and `select` bindings are locals, each in scope from the end of its own statement to the end of its block, case clause or `if`/`for`/`switch` (so `x := x()` still calls the function). The parameters of a `func(x T)` type are not bindings. A bare call through one is `shadowed-by-local`.
  - 0.13.3 fixture `lang-docs/go/f1`: `func run(cb func()) { cb() }` and `for _, step := range fs { step() }` are both `shadowed-by-local`; fixture f10 had bound them to package functions at high.
- **Go's own import-shadowing model** (a substitute for locals that covers only names an import binds):
  - A parameter or receiver named like an import binding shadows it for the function body. It is typed when the parameter type is package-qualified, else `<local>` (record_param_shadows, go.rs:647-691).
  - A `:=` local named like an import shadows it to the end of its block (go.rs:553-575).
  - `FileModules` turns these into `go_shadow` spans (modules.rs:1561-1590), and `resolve_go` defers inside them (modules.rs:2005-2012).
  - Tests `a_parameter_shadowing_an_import_is_scoped_to_the_body` (go.rs:1005), `go_local_shadowing_an_import_alias_is_not_refused` (cross_file.rs tests).
- **Lexical narrowing of nested definitions**: a `name := func(…){…}` closure is visible by name only inside its function, and a call through the local binds the closure its own declaration defines (§02; 0.13.3 fixture `lang-docs/go/f3`). From another file such a closure is never a bare-name candidate (`cross_file::nested_function_ids`, WS33).
- **Positional shadowing**: not applicable or not implemented.
- **Block scope**: tracked for every local through the locals table (above). Receiver *types* remain file-wide for `var`/`New*` readings (§05).
- **Conditional definitions**: build-tag twins are not distinguished (fixture f12, §01). `DefRecord::conditional` is never set.

## 08. Macros, preprocessor & codegen

- Not applicable: Go has no macros or preprocessor. `c_macro`/`rust_macro` are not used.
- **`//go:generate`, `//go:linkname`, `//go:noinline`, `//export`** directives are comments to the grammar and are ignored. Fixture f7: `//export GoCallback` produces no attribute and no FFI link.
- **Generics**: instantiated calls are handled (§03). There is no template expansion.
- **Generated code tagging** (`macro:<name>`): none.
- **`name_mentions`**: never populated by the Go plugin.

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: no Go family. The families are c-abi, pyo3, wasm-bindgen, napi, jni, pinvoke and kv-python (ffi.rs:1-13). All are attribute-driven, and Go declares no attributes.
- **cgo**: not linked in either direction. Fixture f7:
  - `C.add(1, 2)` goes to the external bucket (receiver `C`), with no edge to `native.c`'s `add`.
  - A C call to `GoCallback()` binds to the C prototype in `native.c` (intra-file), not to the Go `//export GoCallback` function.
- **Protobuf descriptor → Go** (`descriptor::link_descriptors`, descriptor.rs): an rpc links to a Go method only when the method name matches case-insensitively **and** the owner names the service (`GreeterServer`, `GreeterImpl`, `UnimplementedGreeterServer` …).
  - Fixture f7: `hello.Greeter.SayHello -> main.GreeterServer.SayHello [low descriptor]`. A lowercase `server.SayHello` is not linked.
  - The module doc's own example (`func (s *server) SayHello`, descriptor.rs:10) would not link under that rule.
  - Tests `rpc_links_to_a_conventionally_named_implementation` (descriptor.rs:193), `a_bare_method_name_match_is_not_enough` (descriptor.rs:225), integration test `a_proto_rpc_links_to_its_go_implementation` (crates/cgg/tests/frameworks.rs:775).
- **Markup code-behind**: not applicable.
- **Go templates** (`html/template` `{{ .Method }}`): not parsed.

## 10. Frameworks & entry points

Rules with `language: "go"` in `crates/cgg-core/src/frameworks/rules.rs` (29). Registrar verbs match case-insensitively (rules.rs HTTP_VERBS comment, test `shape_b_go_router_verb_matches_case_insensitively`, crates/cgg/tests/frameworks.rs:146).

| rule id | line | kind | node | detect (import prefix) | trigger |
|---|---|---|---|---|---|
| gin | 798 | network | yes | github.com/gin-gonic/gin | registrars GET POST PUT DELETE PATCH HEAD OPTIONS Any Handle Use |
| echo | 818 | network | yes | github.com/labstack/echo | registrars GET…OPTIONS Any Add Use |
| fiber | 838 | network | yes | github.com/gofiber/fiber | registrars Get…Options All Add Use |
| chi | 858 | network | yes | github.com/go-chi/chi | registrars Get…Options Handle HandleFunc Method Use |
| net-http | 887 | network | yes | net/http | registrars HandleFunc Handle; methods ServeHTTP |
| temporal | 910 | queue | yes | go.temporal.io/sdk | registrars RegisterWorkflow RegisterActivity ExecuteWorkflow ExecuteActivity |
| cobra | 2365 | cli | — | github.com/spf13/cobra | **gap only** (Run/RunE are struct fields) |
| urfave-cli | 2382 | cli | — | github.com/urfave/cli | **gap only** (Action field) |
| grpc-go | 2399 | network | — | google.golang.org/grpc | **gap only** (implicit service interfaces) |
| beego | 2416 | network | — | github.com/beego/beego, github.com/astaxie/beego | **gap only** (reflection-dispatched controllers) |
| iris | 2433 | network | — | github.com/kataras/iris | **gap only** |
| buffalo | 2450 | network | — | github.com/gobuffalo/buffalo | **gap only** |
| aws-lambda-go | 2467 | network | yes | github.com/aws/aws-lambda-go | registrars Start StartWithContext StartWithOptions StartHandler StartHandlerFunc StartHandlerWithContext (+gap: trust kind always network) |
| gcp-functions | 4829 | network | yes | github.com/GoogleCloudPlatform/functions-framework-go | registrars HTTP CloudEvent Typed (+gap) |
| go-testing | 7631 | **test** | yes | testing | methods Test*Benchmark* Fuzz*Example* TestMain (+gap, see §11) |
| go-encoding | 7648 | lifecycle | **no** (root mark) | encoding, encoding/json, encoding/xml, encoding/gob, gopkg.in/yaml.v2, gopkg.in/yaml.v3 | methods MarshalJSON UnmarshalJSON MarshalText UnmarshalText MarshalBinary UnmarshalBinary MarshalXML UnmarshalXML GobEncode GobDecode MarshalYAML UnmarshalYAML |
| go-signal | 7685 | lifecycle | no | os/signal | **gap only** (channel delivery) |
| go-runtime | 7702 | lifecycle | no | fmt, os, context, sync, errors | **gap only** (func literals, implicit interfaces) |
| gorilla-mux | 8838 | network | yes | github.com/gorilla/mux, …/handlers, …/websocket | registrars HandleFunc Handle |
| grpc-gateway | 8859 | network | — | github.com/grpc-ecosystem/grpc-gateway | **gap only** |
| kratos | 8876 | network | — | github.com/go-kratos/kratos | **gap only** |
| go-kit | 8893 | network | — | github.com/go-kit/kit, github.com/go-kit/log | **gap only** |
| goa | 8910 | network | — | goa.design/goa, goa.design/plugins | **gap only** |
| revel | 8927 | network | — | github.com/revel/revel, github.com/revel/modules | **gap only** |
| martini | 8944 | network | yes | github.com/go-martini/martini, github.com/martini-contrib | registrars Get Post Put Patch Delete Head Options Any NotFound Group |
| hertz | 8967 | network | yes | github.com/cloudwego/hertz | registrars GET…OPTIONS Any Handle Group NoRoute NoMethod |
| asynq | 8987 | queue | yes | github.com/hibiken/asynq | registrars HandleFunc Handle; methods ProcessTask |
| machinery | 9004 | queue | — | github.com/RichardKnop/machinery | **gap only** (map-literal registration) |
| robfig-cron | 9021 | schedule | yes | github.com/robfig/cron | registrars AddFunc AddJob Schedule |

None of these rules sets `attributes`, `base_types` or `string_targets` (all `false`/`NONE` in the extracted specs).

- **Trigger shapes used by Go**:
  - registrar calls with an identifier handler argument;
  - registrar calls with an inline func literal, which get a `handler_at_<line>` node (test `shape_c_go_inline_martini_route_is_an_entry`, frameworks.rs:182);
  - method-name matching (`methods`), e.g. `ServeHTTP` (test `go_servehttp_is_recognised_without_any_implements_declaration`, frameworks/mod.rs).
- **Fixture f5** (gin + net/http + aws-lambda-go + robfig/cron + encoding/json) minted:
  - `gin::get("/users")` → `listUsers`;
  - `gin::post("/inline")` → `handler_at_32`;
  - `net-http::handlefunc("/hf")` → `hf`;
  - `net-http::svc.T.ServeHTTP` → `T.ServeHTTP`;
  - `aws-lambda-go::svc.handle` → `handle` (also test `aws_lambda_go_start_binds_the_handler_value`, frameworks.rs:837);
  - `robfig-cron::addfunc("@every 1m")` → `tick`;
  - `go-encoding` root-marked `T.MarshalJSON` without a node.
- **Helper tables** (rules.rs after line 11627): **no Go entries** in `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for` or `self_module_markers_for`. No Go rule has `detect_paths` or `detect_calls`.
- **Registrar verbs for Go** are the union of the Go rules' `registrars` (`registrar_verbs_for`, rules.rs). They also gate value-ref capture (§03).
- **Implicit interfaces**: only `ServeHTTP` (net-http), `ProcessTask` (asynq) and the encoding family are recognised. `String`, `Error`, `Read`, `Write`, `Close`, `Len`/`Less`/`Swap`, `Scan`, `Value` … are not; this is stated in the `go-runtime` gap. Fixture f5: `T.String` is reported dead.

## 11. Test harness entries

- **`go-testing` rule** (rules.rs:7631, kind `test`, node true): mints **one** node, `<framework-entry>::test::go-testing`, with an `entry` edge to each function matching `Test*`, `Benchmark*`, `Fuzz*`, `Example*` or `TestMain` (case-sensitive prefix patterns). The match is gated to a `_test.go` file or a file importing `testing` (frameworks/mod.rs:1007-1019; test `a_harness_name_counts_only_in_a_test_file_or_one_importing_the_harness`).
  - Fixture f5: entries for `TestA`, `Test_under`, `BenchmarkB`, `FuzzF`, `ExampleE` and `TestMain`. **`Example` (bare) and `Testing` are not entered.**
  - Fixture f11: a `_test.go` file **not** importing `testing` got no harness node. Its `TestOther` was rooted only by `builtin:test-name`.
- **Test file classification**: `*_test.go` (testfile.rs:78; test at testfile.rs:147).
- **Plugin `TestRole`**: `Case` needs an uppercase letter after the prefix; `TestMain` is a `Fixture` (go.rs:950-963).
  - `Test_under` gets **no** `TestRole`, although `go test` runs it. It is still entered by the harness rule's `Test*` pattern (fixture f5).
- **Dead-code name roots** (`is_test_name`, roots.rs:1134-1151): Go follows `go test`'s rule: `Test`, `Benchmark`, `Fuzz` or `Example` followed by anything but a lowercase letter, or a bare `Example`. There is **no file gating**.
  - 0.13.3 fixture `lang-docs/go/f2`: `Example` is a `builtin:test-name` root; `Testing` and `Fuzzy` are not roots; `Test_under` is entered by the harness.
  - `init` and `TestMain` are program-entry names (`LANG_ENTRY`).
- **Testify suites**: methods `func (s *Suite) TestX()` match `Test*` by simple name (match_methods reads `simple_name` regardless of receiver; frameworks/mod.rs:1020). This is by code reading only and was not fixture-tested.
- **Sub-tests and closures**: `t.Run("x", func(t *testing.T){…})`, `f.Fuzz(func…)` and `b.RunParallel(func…)` get no node. Their calls are attributed to the enclosing test (fixture f5: `TestA -> helperT`, high).
- **Stale gap text**: the `go-testing` gap (rules.rs:8187) still says `is_test_name` "only recognises `Test*`" and that Benchmark/Fuzz/Example "are not root-marked". The code (roots.rs:1145-1151) contradicts this.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: Go is not listed (rules.rs; `["python","php","ruby"]`), which is not applicable anyway.
- **`--no-test-entries`**: generic; it turns the harness node into root marks.

## 12. Dead-code analysis

Fixture f5 dead-code JSON `capabilities` for Go:

- `class: degraded`, `max_confidence: medium`;
- visibility `full`, attributes `none`, value_references `full`, dispatch `none`, exports `none`, test_tagging `full`;
- active root rules: `builtin:main`, `builtin:test-name`, `framework:entry-node`, `framework:rule`, `framework:test-harness`.
- **Every Go finding is therefore capped at medium**, with the risks `language-lacks-attributes` and `language-lacks-dispatch-model` (fixture f5 report; unchanged in 0.13.3 fixture `lang-docs/go/f2`, whose capability row is the same).

Roots:

- `main` (universal);
- `init` and `TestMain` (roots.rs:24);
- framework entry nodes and root marks (§10);
- test names (§11).

Exported (uppercase) functions are **not** roots. They carry the risk `public-visibility:public`; unexported ones get the evidence `private-visibility` (fixture f5: `svc.Exported`, `svc.Wire` and `svc.T.String` reported with `public-visibility`).

Signals:

- **`dyn_uses`**: literal names passed to `MethodByName`/`FieldByName` (dynuse.rs:55). Fixture f5: `svc.reflected` carries `name-collides-with-screened-site:reflection`.
  - The plugin's `signals()` does not declare `dyn_uses` (go.rs:40-48), although it extracts them.
- **`unreachable`**: terminators `return`, `break`, `continue`, `goto`, but **not `panic(...)`, `os.Exit`, `log.Fatal`**. Since 0.11.1 the regions are consumed under `--dead-code` (`cgg_resolve::unreachable`; Go regions had never been found because tree-sitter-go nests a block's statements in a `statement_list`): an edge from a site inside one is capped at `medium` and keeps nothing alive. 0.13.3 fixture `lang-docs/go/f2` under `--dead-code`: `return; two()` gives `after -> two` at medium, while `panic("x"); grp()` still gives `pan -> grp` at high.
- **Value refs**: every value position listed in §03, not only registrar arguments. 0.13.3 fixture `lang-docs/go/f1`: `lib.less`, passed to `sortSlice`, has an incoming reference edge.
- **Interface implementations**: under `--dead-code` the `go_iface` edges (§06) keep a satisfying type's methods live, and a method whose type satisfies a standard-library interface (`fmt.Stringer`, `sort.Interface`, …) carries `implements-external-contract`.

Caps:

- `implements-external-contract`: produced for a method whose type satisfies a `GO_STDLIB_INTERFACES` interface (§06).
- `macro-generated`: not applicable.
- Reflection caps at medium, as for every `dynuse` language.

Doubt from unresolved same-name sites (`no-unresolved-site-with-name` evidence) applies as for every language.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/go.txt`**: 986 lines (`wc -l`), loaded at stdlib.rs:186, 227. It mixes:
  - package paths (`archive/zip`, `adler32`);
  - builtins (`append`, `any`);
  - a very broad set of exported stdlib identifiers, including common method names: `Do`, `Handle`, `New`, `Run`, `Add`, `Close` …
- **The table is used as a name screen in two places**:
  1. Unresolved calls whose receiver is a stdlib package go to the stdlib bucket (fixture f1: `fmt.Println`, `os.Exit` → `stdlib_calls`).
  2. **Duck-typed fan-out on an untyped receiver is skipped entirely when the method name is in the table**. Receivers and parameters are typed since WS23, but composite-literal `:=` locals, fields and range variables are not (§05), so workspace methods named `Run`, `Do`, `Handle`, `New` … called through them are **unresolved**:
     - fixture f2/f3: `s.Handle()`, `q.Handle()` → `candidates-in-other-files`;
     - whereas the non-stdlib names `Zap`, `helper` fan out at medium;
     - 0.13.3 fixture `lang-docs/go/f1`: `q := &Server{}; q.Handle()` is `candidates-in-other-files`, while `z.Zap()` fans out at medium.
- **`--include-stdlib`** mints `<stdlib>::fmt::Println` style exit nodes (fixture f1 `stdx.json`).
- **`--include-external`** mints `<external>::…` nodes for every external-bucket site. This includes workspace-internal misses such as `<external>::s::Handle`, `<external>::jobs::Missing` and `<external>::f` (fixture f1), which is misleading.
- **Other tables**: no Go entries in `CAST_INTRINSICS`, `CONSTRUCTING_METHODS` (`new(T)` is not a construction), `MAYBE_TYPE_WRAPPERS`, `NULL_TYPE_MEMBERS`, `TOP_TYPES` or `CALLBACK_TYPE_CONSTRUCTORS` (stdlib.rs:35-160). There is no `rust_types`-like wrapper table for Go.
- **Implicit packages**: none (the builtins `len`, `append`, `make`, `panic` … are plain stdlib names).

## 14. Confidence & certainty

What can yield `high` for Go:

- **Intra-file exact bare calls** (fixture f1 `main -> local`).
- **Same-package bare calls across files** (step 0c `certain:true`, modules.rs `go_same_package`; fixture f9 `three -> a.shared`).
- **`pkg.F()` through a workspace import**, including aliased imports (fixture f1 `jobs.SpawnJob`, `j2.SpawnJob`).
- **`pkg.T` typed receivers**:
  - package-qualified params (`r *jobs.Runner`);
  - package-made locals (`jobs.NewRunner()`, `&jobs.Runner{}`, `jobs.Open(p)`).
  - Fixture f1: all high.
- **Chained `pkg.NewX().M()`** through the declared result type (fixture f1).
- **Typed receiver shapes**: composite literal, type assertion and method expression (fixtures f1, f6).
- **Same-file typed locals**: `var x T`, `x := NewT()` (fixtures f4, f9).

Owner uniqueness for Go is judged per directory (`unique_owner_names`, cross_file.rs:4415). Go is not in `qn_carries_module` (cross_file.rs:4772-4777), so a bare owner path is certain only through `cert.owner` + `cert.anchored`. A Go free function reached as `form.F()` through a local named like the caller's own package is never certain (cross_file.rs:6119-6136; CHANGELOG.md:85).

What forces `medium`:

- duck-typed fan-out on untyped receivers (receivers, same-package params, fields; fixture f4);
- dot-import bare calls (fixture f6);
- embedded-promotion lookups through `bases()` (`certain:false`);
- generic composite-literal receivers (fixture f1 `m.Put` medium);
- build-tag twins (fixture f12).

**False `high` observed** (0.11.0):

- file-wide `var` last-write-wins (fixture f9 `one -> B.Zork`); in 0.13.3 the wrong edge remains but at medium (§05);
- parameter/range-variable shadowing of a same-named package function (fixture f10 `run -> cb`, `loop -> step`); fixed by the WS27 locals table (§07);
- the `New*` naming heuristic (a firm type from a naming convention). Still so in 0.13.3 and it beats the declared result: fixture `lang-docs/go/f4`, `n := NewServer(); n.Zap()` with `func NewServer() *Other` binds `Server.Zap` at high.

Since 0.11.1 also `high`: receiver and same-package parameter types, same-package call results, member-read aliases through a typed variable (§05, §03).

Framework entry edges are `low`; descriptor edges are `low`. The mermaid `?` marker applies generically, on arrows with no `high` site.

## 15. Dynamic use & reflection

- **Reflection by literal name**: `reflect.Value.MethodByName("m")` and `FieldByName("f")` record a `DynUse` (dynuse.rs:55; extracted only under dead-code signals, go.rs:114-116). The effect is suppression only: same-named findings are capped at medium, and no edge is created (fixture f5, `svc.reflected`).
- **Not covered**:
  - `reflect.ValueOf(x).Method(i)`;
  - `Call` on method sets;
  - `plugin.Open`/`Lookup("Sym")` (Go's module-load analogue; `DYN_MODULE_LOADERS` is `require`, `importScripts`, `import_module` only, facts.rs:165);
  - `//go:linkname`;
  - string-keyed dispatch maps (`map[string]func(){…}`) produce reference edges to each value (§03) but no call edge from the dispatch site.
- **The `signals()` manifest under-declares `dyn_uses`** (§12).

## 16. Known gaps & limitations

From code comments and rule gaps:

- The plugin header says "embedded-struct method promotion, interface method dispatch, and generics are not modeled" (go.rs:19-21). The comment is stale: generics are partly handled (§03), embedding is recorded as bases and interface dispatch is modeled by `go_iface` (§06).
- `go-runtime` gap (rules.rs:7702): no nodes for anonymous func literals outside registrars, func values in fields/maps/slices, and implicit interfaces.
- Gap-only frameworks:
  - CLI: `cobra`, `urfave-cli`;
  - RPC and code-generated: `grpc-go`, `grpc-gateway`, `kratos`, `go-kit`, `goa`;
  - web: `beego`, `iris`, `buffalo`, `revel`;
  - other: `machinery`, `go-signal`.
- `aws-lambda-go` and `gcp-functions` gaps: the trust kind is always `network`; parameter (event) types are not read.
- `go-testing` gap text is **stale** relative to roots.rs (§11).

From the CHANGELOG:

- Package-variable function aliases (`var New = sdk.New`) are refused rather than followed (CHANGELOG.md:1320-1321; still so in 0.13.3, §03).

Found while cataloguing (fixtures; status as of 0.13.3):

1. Same-package composite literals / `new(T)` in `:=`, package-qualified and grouped `var`s, multi-name `var`s past the first, type-switch and range bindings, and struct fields are untyped (f2, f15; `lang-docs/go/f1`, `f2`). Receivers and same-package parameters are typed since WS23.
2. A file-wide last-write-wins `var` typing produces a **wrong** edge, now at medium (f9; `lang-docs/go/f1`).
3. Fixed (WS27): parameters and range variables shadow package functions through the locals table.
4. The stdlib name screen (986 names incl. `Run`, `Do`, `Handle`, `New`) silently blocks fan-out for untyped receivers. Workspace methods with those names are unresolved (f2, f3; `lang-docs/go/f1`).
5. Fixed (WS22, WS31): interface members are tagged abstract, and `--dynamic-dispatch` adds interface→impl edges through Go method sets.
6. Fixed (WS26): functions passed as values are value references.
7. `a, b := func(){}, func(){}` mints a node named `a, b` (f13; still so in `lang-docs/go/f1`). Package-level and `var`-declared func literals are not extracted (f1, f13; `lang-docs/go/f1` `vlit`).
8. An external dot import lets bare calls reach any workspace directory (f14; `lang-docs/go/f2`).
9. cgo is not linked in either direction (f7; not re-checked).
10. Build-tag twins are both bound (f12; not re-checked).
11. `--include-external` labels workspace misses as `<external>` (f1; not re-checked).
12. `FileFacts::unreachable` is consumed under `--dead-code` since 0.11.1; `panic` is still not a terminator (`lang-docs/go/f2`).
13. Fixed: `is_test_name` follows `go test`'s rule (`Testing`, `Fuzzy` are not roots). It still has no file gating.
14. The proto descriptor link needs a service-named owner; a lowercase `server` impl is not linked (f7; not re-checked).

- **Ignored tests**: none found for Go. A grep of `#[ignore]` near Go tests in go.rs, modules.rs and cross_file.rs found none.

## Summary scorecard

| section | support | one-line note |
|---|---|---|
| 01 Detection & parsing | full | `.go` only, one grammar, `go.mod` read for module/replace; no build-tag awareness |
| 02 Callables extracted | partial | funcs, methods (generic owners), `abstract` interface specs, `:=` literals, inline handlers; no package-level/`var` literals, bogus multi-assign node, no nested QNs |
| 03 Call-site forms | full | bare/selector/peeled shapes/generic instantiation/method expressions/same-package chains; value refs in every value position; member-read aliases; package-var aliases not followed |
| 04 Imports & module resolution | full | go.mod + GOPATH, aliases, dot/blank, `_test` exclusion, external + not-in-imported-module refusals; dot-import of external leaks |
| 05 Type inference & receiver typing | partial | receivers, all params and call results typed; fields, composite `:=`, qualified/grouped `var`, type switches, range vars untyped; file-wide `var` gives wrong medium edges |
| 06 Inheritance, dispatch & overloading | partial | embedded fields/interfaces as bases; `go_iface` method-set dispatch under `--dynamic-dispatch`/`--dead-code`; not in INHERITANCE_LANGS |
| 07 Scoping & shadowing | full | locals table (params, receivers, `:=`, `var`, range, switch/select bindings) plus import-name shadowing; closures bind their own declaration |
| 08 Macros, preprocessor & codegen | n/a | Go has no macros; directives ignored |
| 09 FFI & cross-language links | minimal | proto rpc → service-named Go owner only; no cgo / `//export` |
| 10 Frameworks & entry points | partial | 29 rules: 15 enumerate (gin/echo/fiber/chi/net-http/gorilla/martini/hertz/asynq/temporal/cron/lambda/gcp/testing/encoding), 14 gap-only |
| 11 Test harness entries | full | go-testing harness node for Test/Benchmark/Fuzz/Example/TestMain; name roots follow `go test` but are not file-gated; gap text stale |
| 12 Dead-code analysis | partial | class degraded, max medium; exported not rooted; reflection screen, value refs, interface implementers and unreachable regions work; `panic` not a terminator |
| 13 Builtins & stdlib tables | partial | 986-name go.txt; broad screen blocks fan-out of common method names; no cast/wrapper tables |
| 14 Confidence & certainty | partial | many high paths; file-wide `var` wrong edges now medium; `New*` heuristic still firm |
| 15 Dynamic use & reflection | minimal | MethodByName/FieldByName only; signal under-declared |
| 16 Known gaps & limitations | n/a | 14 fixture-confirmed gaps (4 fixed since 0.11.0) plus rule/CHANGELOG gaps listed |
