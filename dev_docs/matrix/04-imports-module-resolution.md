# 04. Imports & module resolution

This category covers how cgg uses a source file's import-like statements to decide which definitions a call can reach. That includes `import`/`include`/`require`/`use`/`open`/`source`/`load` and the alias, selective-list, wildcard and re-export forms of each. It also covers four other things: the package or namespace identity a file declares (`FileFacts::packages` and its equivalents), manifests read to name workspace modules (`plugins/manifest.rs`), refusing calls into modules outside the tree (`heads.rs` `WorkspaceVocab`/`FileHeads`, and the `Scoped::External` paths in `modules.rs` and `libscope.rs`), and refusing a call through an in-tree module that does not declare the callee (`not-in-imported-module`, `Scoped::Refused`). Type inference, receiver typing and inheritance are out of scope (sections 05–07), and so are FFI and descriptor→host links (section 09). Where an import decides *which copy* a call binds, that is in scope, because the import is the evidence.

Sources: section 04 and the Summary row 04 of every `dev_docs/languages/<lang>.md`, spot-checked against `crates/cgg-resolve/src/{cross_file,heads,modules,libscope}.rs` and `crates/cgg-core/src/lang_family.rs`, plus the fixtures listed under "Fixtures run for this matrix".

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | full: the language's ordinary forms work and are tested or fixture-verified |
| 🟡 | partial: some forms work, others are dropped, or it works with known wrong edges |
| ⬜ | none: the language has the concept and cgg does not model it |
| — | n/a: the concept does not exist in the language, or is meaningless for it |

### Columns

| Column | Capability |
| --- | --- |
| **Forms** | The plugin records ImportRecords for the language's import or include statements. |
| **Alias** | A local rename (`as`, `=>`, `:as`, `local m = require`, `using A = …`, a module abbreviation) binds the local name to the module or member. |
| **Select** | Selective lists narrow what comes into scope: `from m import a`, `show`/`hide`, `only:`, `:refer`, `hiding`, `-import(M,[f/1])`, a symbol list in a `load`, Exporter lists. |
| **Open** | Wildcard or open forms put a module's members in bare-call scope: `*`, `_`, `open`, `using namespace`, `use M`, `import M`, static wildcards. |
| **Chains** | Re-exports or transitive includes are followed: `pub use`, barrels, `export … from`, Dart `export`, includes of includes. |
| **Scoped** | A call written through an import (`mod.f()`, `alias/f`, `m:f`, `pkg.F()`, an include) is looked up in the module or file the import names. |
| **Anchor** | When several files define the same name, the import decides the copy, and the call never binds a file that was not imported. |
| **Pkg** | Package or namespace identity is recorded (`FileFacts::packages`, or an equivalent record that the resolver reads). |
| **Manifest** | Build or package manifests are read to name workspace modules: `Cargo.toml`, `package.json`, `go.mod`, `pubspec.yaml`, `build.zig`, and so on. |
| **Ext** | External-module refusal: a call through, or naming, an import of an out-of-tree module is filed `external-module` (or `Scoped::External`) instead of being name-matched onto a workspace definition. |
| **NIM** | `not-in-imported-module`, or the equivalent `Scoped::Refused`: a call through an in-tree module that lacks the name is refused, not bound elsewhere. |
| **BareGate** | A cross-file bare call reaches what the language actually puts in scope, neither more nor less. The usual failure is the step-1d global fallback (`cross_file.rs:6015-6030`): with any import present it binds a name that has ≤3 candidates at medium, and with no import it binds nothing. |
| **XLang** | Imports cross language boundaries within a family: C/C++/ObjC through the include closure, TS→JS through imports, Elixir→Erlang, KV→Python. |

## The matrix

| # | Language | Forms | Alias | Select | Open | Chains | Scoped | Anchor | Pkg | Manifest | Ext | NIM | BareGate | XLang | Notes (single most important gap) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | rust | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | 🟡 | 🟡 | — | A glob `use a::*` binds only at medium. Crate identity comes from a synthetic `crate-root` import, not `packages`. There is no `not-in-imported-module` record: path receivers fail closed through step 4c instead (rust.md §04). |
| 2 | python | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | 🟡 | 🟡 | — | A bare call to a name that was never imported still binds by name at medium (fixture f11 `run()`). `pyproject`/`setup.cfg` entry points are not read. |
| 3 | kivy | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | 🟡 | `#:import`/`#:include` are ignored. KV→Python links by simple name only (`ffi.rs:317-321`). |
| 4 | javascript | ✅ | ✅ | ✅ | — | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 | 🟡 | 🟡 | Dynamic `import()` and `require(var)` are unmodelled. tsconfig `paths` targets are not read, only the keys. A missing member of a workspace package is filed external, not `not-in-imported-module`. |
| 5 | typescript | ✅ | ✅ | ✅ | — | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 | 🟡 | 🟡 | `namespace`/`declare module` blocks are not module scopes. `lib.missing()` through an in-tree module leaves no `not-in-imported-module` record. Aliased imports leave stale external records. |
| 6 | go | ✅ | ✅ | — | 🟡 | — | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — | A dot-import of an *external* package opens any-directory bare binding (fixture f14 `Describe()`). Package-variable aliases (`var New = sdk.New`) are not followed. |
| 7 | java | ✅ | — | ✅ | ✅ | — | 🟡 | ✅ | ✅ | ⬜ | ✅ | ⬜ | ✅ | ⬜ | No JVM family: Java↔Kotlin/Scala/Groovy calls never link (`lang_family.rs:32`). Maven/Gradle are not read. |
| 8 | kotlin | ✅ | 🟡 | ✅ | ✅ | — | 🟡 | ✅ | ✅ | ⬜ | ✅ | ⬜ | 🟡 | ⬜ | An import alias of an object or object member (`import u.Q.qf as aliased`) binds nothing. Kotlin→Java does not link. |
| 9 | c | 🟡 | — | — | — | ✅ | ✅ | ✅ | — | ⬜ | ⬜ | — | 🟡 | ✅ | A file with no resolvable `#include` gets no cross-file binding at all (fixture f6). `#include MACRO` is not recorded. No heads.rs refusal. |
| 10 | cpp | 🟡 | 🟡 | ⬜ | ⬜ | ✅ | ✅ | 🟡 | 🟡 | ⬜ | ✅ | ⬜ | 🟡 | ✅ | `using namespace X;` and `using X::f;` are not recorded. A namespace alias to a workspace namespace does not narrow (`bl::util` binds two). Names outside the closure still bind by name. |
| 11 | csharp | 🟡 | ✅ | ✅ | ✅ | — | ✅ | ✅ | ✅ | ⬜ | ✅ | ⬜ | ✅ | ⬜ | `global using static` is parsed as a plain `using` and then rejected (`csharp.rs:428`, `heads.rs:875`). `.csproj` and `extern alias` are not read. |
| 12 | bash | 🟡 | — | — | — | ⬜ | 🟡 | ⬜ | — | — | — | — | ⬜ | — | A sourced name binds an arbitrary same-named copy at **high** (wrong-file). Sourcing is not transitive. `$VAR` paths are unresolved. |
| 13 | ruby | 🟡 | — | — | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | 🟡 | — | `require_relative` binds the *non-required* copy at high: the same `by_qn` bug as bash, verified for this matrix. There is no heads.rs, `modules.rs` or Gemfile support. |
| 14 | swift | 🟡 | — | ⬜ | 🟡 | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | Same-module bare calls bind only if the file has some unrelated `import` (f2). `Module.f()` never binds. `Package.swift` is not read. |
| 15 | lua | 🟡 | ✅ | — | — | — | 🟡 | ✅ | — | ⬜ | ✅ | 🟡 | 🟡 | — | `util.sub.deep()` and inline `require("x").f()` fall back to by-name binding. `package.path` and `dofile` are not read. |
| 16 | php | 🟡 | ✅ | ✅ | — | 🟡 | ✅ | 🟡 | 🟡 | ⬜ | ✅ | ⬜ | ✅ | — | A braced `namespace A { … }` is mis-recorded or dropped (`php.rs:250-283`). `require` is matched by basename only. Composer PSR-4 is not read. |
| 17 | dart | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | — | The external label names the wrong library for a `show`/`hide`-filtered name. `u.missing()` is filed external, not `not-in-imported-module`. |
| 18 | scala | ✅ | 🟡 | ✅ | ✅ | — | 🟡 | 🟡 | 🟡 | ⬜ | ✅ | ⬜ | 🟡 | ⬜ | Renamed-import receivers (`{Tools => T}; T.tool()`) are not followed. Scala 3 top-level defs are unreachable from other files. |
| 19 | hcl | ⬜ | — | — | — | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | — | No import records at all: a `module` `source`, Terragrunt `include` and `templatefile` never bind, so there are no cross-file edges. |
| 20 | zig | ✅ | ✅ | — | ⬜ | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ | ✅ | ✅ | — | `usingnamespace` members are not visible. `build.zig.zon`, `b.dependency()`, `createModule` and `addImport` are not read. |
| 21 | objc | 🟡 | ⬜ | — | — | ✅ | 🟡 | 🟡 | — | ⬜ | ⬜ | — | 🟡 | ✅ | Message sends are not scoped by the include closure (`[eng start]` fans out to a file nothing imports). `@import M;` is not recorded. |
| 22 | r | 🟡 | ⬜ | ⬜ | 🟡 | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | `pkg::f` is never checked against the package: `dplyr::filter` binds a workspace `filter` at high. `source()` binds the wrong copy at high (verified). |
| 23 | groovy | 🟡 | ⬜ | ✅ | ⬜ | — | 🟡 | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | `import X as Y` records the wrong alias. `import c.*` loses the `*`. With no external refusal, an external `o.go()` binds an unrelated workspace `go` at medium. |
| 24 | julia | 🟡 | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | `using A` is inert (no resolver arm), as are `import A: f` and `using .A`. `A.fa()` never binds. `include()` is not an import. |
| 25 | perl | 🟡 | — | ⬜ | 🟡 | ⬜ | 🟡 | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ✅ | — | Exporter lists are ignored: `use A; not_exported()` binds high. `require M` never maps to `M.pm`. There is no package→file mapping. |
| 26 | elixir | ✅ | ✅ | ⬜ | ✅ | — | ✅ | 🟡 | 🟡 | ⬜ | ✅ | 🟡 | 🟡 | 🟡 | `import M, only:/except:` filters are ignored. Imports are file-scoped, not module-scoped (fixture f5). |
| 27 | erlang | ✅ | — | ⬜ | 🟡 | 🟡 | ✅ | ✅ | — | ⬜ | 🟡 | ⬜ | ✅ | 🟡 | The `-import(M,[f/1])` list is discarded, so binding is arity-blind and over-broad (f1 `bfun/2`). A known module lacking `f` is filed external, not refused. |
| 28 | fortran | 🟡 | ⬜ | ⬜ | ✅ | ⬜ | 🟡 | 🟡 | — | ⬜ | ⬜ | ⬜ | ⬜ | — | A file with no `use` never reaches another file, which leaves F77-style external subroutines unlinked. `include`/`#include` are not recorded. |
| 29 | clojure | 🟡 | 🟡 | ⬜ | 🟡 | — | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | 🟡 | 🟡 | — | `:refer` is ignored, so a bare require opens everything. Prefix lists, `:as-alias`, top-level `(require …)` and `-`→`_` file munging are missing, which gives wrong edges (p5). |
| 30 | haskell | 🟡 | 🟡 | ⬜ | ✅ | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | `hiding` and import lists are ignored, giving wrong high edges (f2 `viaHidden`). Qualified call sites are never emitted, so `import qualified … as` is unused. |
| 31 | ocaml | 🟡 | ⬜ | — | 🟡 | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | — | A qualified `B.foo`, the dominant call form, never resolves. `include M`, local open `M.(e)` and module aliases are missing. |
| 32 | powershell | 🟡 | — | ⬜ | 🟡 | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | Backslash paths (`.\lib\x.ps1`, the norm) never resolve on POSIX. A dot-sourced name binds the wrong copy at high (verified). Export lists are ignored. |
| 33 | solidity | 🟡 | ⬜ | ⬜ | — | ⬜ | 🟡 | ⬜ | — | ⬜ | ⬜ | ⬜ | 🟡 | — | `import "./A.sol"` binds a same-named free function in an un-imported file at high (verified). Aliases, symbol lists, transitive imports and remappings are missing. |
| 34 | fsharp | 🟡 | ⬜ | — | 🟡 | — | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | `open M` works only when `M` happens to match a file stem. Module abbreviations, `#load` and `.fsproj` compile order are missing. |
| 35 | starlark | 🟡 | ⬜ | ⬜ | — | — | 🟡 | ⬜ | — | ⬜ | ⬜ | ⬜ | ✅ | — | A loaded name binds the copy in an un-loaded file at high (t2/t3). The symbol list and aliases (`h2 = "x"`) are dropped, and `@repo` labels are not refused. |
| 36 | cmake | 🟡 | — | — | — | 🟡 | 🟡 | ⬜ | — | — | ⬜ | — | ⬜ | — | An included name binds the wrong-copy at high (t3). `find_package` does not try `FindFoo.cmake`/`FooConfig.cmake`. `${}` paths are unresolved. Transitive includes need a literal `.cmake`. |
| 37 | nix | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | — | `h = import ./x.nix; h.f` is not tied to the file. A bare `f` binds high without `with`, which is a false high. `default.nix`, `callPackage`, `imports = [ … ]`, `inherit` and `with` are missing. |
| 38 | verilog | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | `` `include `` and SV `import pkg::*` are not recorded, although `cross_file.rs:2123-2140` has a Verilog arm. A cross-file instantiation never binds. |
| 39 | vhdl | 🟡 | — | ⬜ | 🟡 | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | There is no library or design-unit→file model: `use work.all` only opens the by-name gate. Binding is case-sensitive (`adder` ≠ `Adder`). |
| 40 | asm | ⬜ | — | — | — | ⬜ | ⬜ | ⬜ | — | — | — | — | ⬜ | — | `.include` is ignored, and global labels in other `.s` files never bind (`has_imports` gate). asm↔C goes through FFI (§09), not imports. |
| 41 | smithy | ⬜ | — | ⬜ | — | — | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | `use a#Thing` is ignored, so every same-named shape (≤4) binds. The namespace is not used. |
| 42 | proto | ⬜ | — | — | — | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | 🟡 | — | `import "x.proto"` is not read and there is no package narrowing: a nested `a.Outer.Inner` binds an unrelated `b.Inner`. |
| 43 | graphql | — | — | — | — | — | — | ⬜ | — | ⬜ | — | — | 🟡 | — | There is no import concept. The global ≤4 by-name cap is hard-coded and ignores `--fanout-cap 0`. Two schemas in one tree cross-link. |
| 44 | openapi | ⬜ | — | — | — | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | 🟡 | ⬜ | The file part of a `$ref` (the real import) is ignored, and a local `#/…` is not pinned to its own document. |
| 45 | asyncapi | ⬜ | — | — | — | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | 🟡 | ⬜ | The `$ref` file path is ignored. The lookup key is `(lang, name)`, so a ref never reaches an OpenAPI schema. |
| 46 | lean | 🟡 | ⬜ | ⬜ | ✅ | ⬜ | 🟡 | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | The current `namespace` is not auto-opened, so a bare `go` inside `namespace B` is unresolved. A qualified `B.foo` binds without any import. Selective `open` forms are missing. |

### Per-cell evidence

Every cell comes from the language doc's section 04 unless a fixture or code line below overrides it. Here are the non-obvious calls.

- **rust Open 🟡**: rust.md §04, which shows a glob `pub use inner::*` binding only at medium.
- **rust Pkg 🟡**: rust.md §04. It uses the synthetic `crate-root` import (rust.rs:68-74), not `packages`.
- **rust NIM 🟡**: rust.md §04 says `modules.rs` has no Rust branch. Step 4c refuses external path heads, and per CLAUDE.md a Rust path receiver is "never duck-typed". There is no explicit refusal for an in-workspace module that lacks the name (unverified).
- **python NIM 🟡**: python.md §04, fixture f11. `lib.nothere()` against a workspace module was minted `<external>::lib::nothere`. The refusal is recorded only when another callable carries the name (`cross_file.rs:2810-2827`).
- **javascript/typescript Manifest 🟡**: javascript.md and typescript.md §04, which cite CHANGELOG.md:1324-1326. tsconfig `paths` keys are read but their targets are not.
- **javascript/typescript XLang 🟡**: typescript.md §04, fixture f15. An imported `.js` resolves high, but an un-imported bare call never reaches `.js`, because the JS/TS family is commented out at `lang_family.rs:33`.
- **go Open 🟡**: go.md §04, fixtures f6 and f14. A dot import binds at medium, and an external dot import leaks.
- **java Scoped 🟡**: Java has no `ModuleIndex` path (`modules.rs:189-191` indexes js/ts/python/go/lua only). Anchoring comes from `OwnerFacts`/`narrow_owner_paths` (cross_file.rs:4623-4643, 4835-4860) instead.
- **kotlin BareGate 🟡**: the member-beats-import rule works (cross_file.rs:5348-5375). Same-package top-level functions without an import are unverified.
- **c Ext ⬜**: `external_head` returns `None` for any language outside `python|javascript|typescript|go|java|kotlin|scala|csharp|cpp` (`heads.rs:660-673`, read for this matrix).
- **c Forms 🟡**: c.md §04. `#include MACRO` is not recorded.
- **cpp Pkg 🟡**: `cpp_scopes` is built from QNs, field owners and aliases (heads.rs:168-205). It is used as vocabulary, not as `packages`.
- **csharp NIM ⬜**: `using` names namespaces, not files. A namespace in the tree that lacks a member has no refusal path (csharp.md §04).
- **bash/ruby/r/powershell/solidity/starlark/cmake Anchor ⬜**: see Parity gap 1. This was verified for this matrix for ruby, r, powershell and solidity, and by the language docs for bash (f10, f12), starlark (t2, t3) and cmake (t3).
- **ruby Chains ⬜**: the `"require"` arm calls `collect_include_defs` with depth 4 (cross_file.rs:1846-1876), but the recursion only follows `include`/`system-include` (cross_file.rs:3507-3510). See Doc corrections.
- **ruby Ext 🟡**: ruby.md §04, fixture p5. A capitalised unknown receiver ends up external, but through the name screen, not an import table.
- **lua Scoped 🟡**: lua.md §04. `util.f()` binds high, but `util.sub.deep()` falls back to by-name binding (`modules.rs:1671-1673`).
- **lua NIM 🟡**: the code returns `Scoped::Refused` (`modules.rs:1692-1694`), but the fixture p1 CLI audit showed `no-candidate-in-file`.
- **php Chains 🟡 / Anchor 🟡**: php.md §04. The include closure is matched by basename to depth 8 (libscope.rs:2004-2088).
- **dart NIM 🟡**: dart.md §04, fixture f2. `h.secret()` is refused, but `u.missing()` is filed external.
- **zig Manifest 🟡**: zig.md §04. Only `addModule(` is read from `build.zig` (manifest.rs:93-133).
- **elixir Anchor 🟡**: there is no fixture for copies of one Elixir module (unverified).
- **elixir XLang 🟡 / erlang XLang 🟡**: Elixir→Erlang `:m.f()` binds high (libscope.rs:1095-1108), but Erlang→Elixir (`'Elixir.A':a()`) is external (elixir.md §04, f5).
- **erlang Chains 🟡**: `-include` has kind `include` and so is followed transitively. `-include_lib` has no resolver arm (erlang.md §04, f11 medium).
- **erlang Ext 🟡**: an unknown module returns `None` (cross_file.rs:5587-5589) and is then classified external by the name screen, not by an import table.
- **perl BareGate ✅**: perl.md §04. A bare call reaches only the caller's package or a `use`d package (cross_file.rs:4159-4162, 5112-5117). The over-permissive Exporter behaviour is filed under Select.
- **ocaml BareGate ✅**: ocaml.md §04, f4. A file with no `open` does not bind across files, which matches OCaml.
- **starlark BareGate ✅**: starlark.md §04. A file with no `load` stays unresolved.
- **proto Ext 🟡**: well-known types are filed external by the name screen (proto.md §04, fixture a).
- **descriptor BareGate 🟡**: the global `(lang, simple_name)` arm binds 1–4 candidates at medium (cross_file.rs:5222-5242).

## Parity gaps

Ranked by how many languages a gap affects and how many wrong or high edges it produces. Effort: S is under a day and local to one arm or function, M is a few days or a new resolver path, L is a cross-cutting design change.

1. **Include-style imports bind an arbitrary same-named copy at `high` in every non-C language.** The languages are bash, ruby, r, powershell, solidity, starlark and cmake, and probably nix, lua (legacy reach) and perl `require`, which are unverified.
   - *Comparable:* the C family scopes these sites with `c_include_copies` (cross_file.rs:5712-5744). Every other language takes `lookup_with_reexports(lang, qn, by_qn, …)` (cross_file.rs:5746), and `by_qn` "keeps an arbitrary one". These languages' QNs are bare names, so the lookup cannot tell the copies apart.
   - *Evidence:* fixture `wc`. Each language had `main` importing `lib/X` with a decoy copy in `zz/`. In all four languages tested, the edge went to `zz/`'s copy at **high**, resolver `cross-file:imports`: `main@main.rb -> dup_fn@zz/two.rb`, `mainr@main.R -> dupr@zz/u2.R`, `Main@main.ps1 -> Get-Dup@zz/h2.ps1` and `C.m@C.sol -> dsol@zz/B.sol`. After renaming `zz/` to `aa/` (fixture `wc2`), all four bound `lib/`, the right file. So the choice is the path-sorted last copy, and it has nothing to do with the import.
   - *Fix:* have `collect_include_defs` record `(qn, FileId)`, or generalise `c_include_copies` to any include-style kind. **S–M.**

2. **External-module refusal is limited to 9 languages.** `heads.rs:660-671` covers python, javascript, typescript, go, java, kotlin, scala, csharp and cpp. Six more have an in-resolver equivalent: rust (step 4c), php (`PhpIndex`), dart, zig and elixir (`libscope`), and lua (`modules.rs`).
   - *Missing where the concept clearly exists:*
     - **groovy**, unlike java and kotlin. Fixture f6 in groovy.md: an imported `org.external.Other` → `o.go()` bound an unrelated workspace `p.A.go` at medium.
     - **r**: `dplyr::filter` and `mypkg::pkgfn` bind workspace functions at **high** (r.md f3, f2).
     - **swift**, **objc** (framework classes), **ruby**, **perl**, **haskell**, **ocaml**, **fsharp**, **julia**, **clojure** (`:import`), **fortran** (intrinsic modules), **solidity** (remapped imports), **starlark** (`@repo//`) and **erlang** (only through the name screen).
   - *Effort:* Groovy is **S** (join the JVM arm and record `package` into `packages`). R `pkg::` is **S–M**. The rest are **M** each.

3. **Includes are not transitive for the include-like kinds other than `include`/`system-include`.** The recursion at cross_file.rs:3507-3510 follows only those two kinds.
   - Bash `source` (bash.md f3), PowerShell dot-source (powershell.md), Ruby `require`, R `source` (r.md f2: the second-level file is reached only at medium by name) and Solidity `import` are each followed one level only. Solidity makes imported symbols visible transitively (solidity.md f6).
   - CMake recursion passes the raw path without the `.cmake` retry (cmake.md t8).
   - *Comparable:* C, C++, ObjC and Erlang `-include` are transitive to depth 8. **S.**

4. **Module-qualified calls never resolve in OCaml, Julia, Haskell, Swift and F#**, and in F# only when there is an `open` plus a stem match.
   - *Comparable:* Elixir (`resolve_elixir`), Clojure (`alias/f` exact, high), Erlang (`m:f` exact), Lua, Python and Go all bind `M.f` in the named module.
   - *Evidence:* ocaml.md §04 (`B.foo` lands in the external bucket, "the dominant OCaml call form"), julia.md §04/§03 (`A.fa(3)`), haskell.md §04 (qualified sites never emitted), swift.md f13 (`Helpers.util()`) and fsharp.md f3/f9.
   - The root cause is shared: these plugins' QNs do not carry the module (§02). **M–L.**

5. **No JVM (or C#/F#, or JS/TS) language family.** `lang_family.rs:32-34` is commented out, so Java↔Kotlin↔Scala↔Groovy and C#↔F# calls in one tree never link.
   - *Comparable:* C/C++/ObjC has a family with qualified-name retry (cross_file.rs:7363-7371).
   - *Evidence:* java.md f8, kotlin.md f9 and scala.md f6. In scala.md f6 the Java methods became dead-code findings. **M.**

6. **Selective import lists are ignored, which gives wrong or over-broad edges.**
   - Haskell `hiding` binds the hidden name at **high** (haskell.md f2).
   - Erlang `-import(M,[f/1])` is arity-blind (erlang.md f1, f3).
   - Clojure `:refer` is ignored (clojure.md p3).
   - Elixir `only:`/`except:` are ignored (cross_file.rs:2066-2071).
   - Fortran `only:`, the Starlark symbol list, Solidity `{A as B}`, Perl Exporter `@EXPORT`/`qw()`, Lean `open A (x)`/`hiding`, Julia `import A: f` (not even recorded) and R `@importFrom`/`box::use` are not read either.
   - PowerShell `Export-ModuleMember`/`FunctionsToExport` (export-side) is ignored too.
   - *Comparable:* Python `from m import a`, Dart `show`/`hide` (refuses filtered names) and Scala selectors/`D => _` all honour the list. **M** total, S per language.

7. **The step-1d gate is keyed on "has any import", not on language scoping** (cross_file.rs:6015-6030). Two failure directions:
   - *Under-binding* for languages whose names are global or module-implicit: Swift same-module calls (swift.md f2), F77 Fortran (fortran.md d), CMake functions (cmake.md t2), Bash without a resolvable source, HCL, Verilog module instantiation, asm labels (all `candidates-in-other-files`) and C without an include (c.md f6).
   - *Over-binding*: one unrelated import opens workspace-wide binding of ≤3 candidates, as with R `library(foo)` (r.md f8b), PowerShell `Import-Module Nonexistent` (powershell.md f4), Python (python.md f11) and JS (javascript.md f2, f7).
   - *Comparable:* Go (same-package, `go_same_package`), Erlang, OCaml, Starlark and Zig get bare-call scoping right. A per-language "global namespace" flag, or a same-target or same-module rule, is **M**.

8. **Alias forms broken or unrecorded.**
   - Groovy `import X as Y` records `Y = X` (groovy.rs:220).
   - Scala renamed receivers `{Tools => T}; T.tool()` are not followed (scala.md f1).
   - Kotlin object and member aliases bind nothing (kotlin.md f1, f8).
   - Solidity `import * as L`/`import "x" as L`: the alias is dropped (solidity.rs:126).
   - Nix `h = import ./x.nix`: the alias is always empty (nix.rs:199).
   - Fortran renames, Julia `import A.f as g`, OCaml `module X = B`, F# `module U = …`, C++ namespace aliases to workspace namespaces (cpp.md f3: binds two candidates), Clojure `:as-alias`, R `box::use(alias = pkg)` and Lean `renaming` are also unhandled.
   - *Comparable:* Python, JS/TS, Go, C#, Dart, Elixir, Lua and Zig aliases all bind high. **S** per language.

9. **Manifest reading is limited to Rust, JS/TS, Go, Dart and Zig** (plugins/manifest.rs).
   - Missing where a manifest names modules: Python (`pyproject`, which leaves the `py-setuptools-entrypoints` rule a declared gap at rules.rs:6900), Java/Kotlin/Groovy (Gradle/Maven), PHP (composer PSR-4), Solidity (`remappings.txt`/`foundry.toml`, solidity.md f5 unresolved), Elixir (umbrella `apps/*`), Swift (`Package.swift`), Haskell (`.cabal`), Julia (`Project.toml`), Erlang (`rebar.config`), PowerShell (`.psd1` `RootModule`), C# (`.csproj` `RootNamespace`), Zig (`build.zig.zon`) and Bazel (`MODULE.bazel`).
   - **M** each. Solidity remappings and PHP PSR-4 have the most resolution impact.

10. **`not-in-imported-module` coverage is uneven.**
    - It is produced by `modules.rs` for js/ts/python/go/lua (`modules.rs:189-191`) and by libscope for dart/elixir/zig.
    - Erlang `m:f` with module `m` present but no `f` returns `None` and is filed external (cross_file.rs:5587-5589). Elixir's equivalent `:m.f()` returns `Scoped::Refused` for the same case.
    - Clojure `alias/f` that misses is silently dropped, with no unresolved record (clojure.md p2).
    - Across languages, the refusal is recorded only when some other callable carries the name (cross_file.rs:2810-2827). Otherwise the site goes to the external bucket, including a workspace module that lacks a member (python.md f11, javascript.md f2, dart.md f2 L16).
    - **S.**

11. **The namespace and package record is missing or non-standard.**
    - Groovy never writes `packages`, although Java and Kotlin do (groovy.md §04). That blocks gap 2's cheap fix.
    - Scala stores `scala-package` as an import record (heads.rs:126-140).
    - PHP stores `kind: "package"` only, and braced namespaces are broken (php.rs:250-283).
    - Elixir, Clojure, Haskell, OCaml, Lean, Smithy and Proto have namespaces that the resolver does not use for narrowing. Proto binds `a.Outer.Inner` to an unrelated `b.Inner` (proto.md c).
    - **S–M.**

12. **C++ `using namespace X;` / `using X::f;` and C++20 modules are not recorded** (no `using_directive` arm in cpp.rs). The C# `using`/`using static` equivalents are fully modelled. **S–M.**

13. **Descriptor languages ignore their own imports.**
    - Proto `import`, Smithy `use` (both copies bind in smithy.md f), and the OpenAPI/AsyncAPI `$ref` file part (a local `#/` binds another document's `Pet`, openapi.md a) are all unused.
    - The ≤4 cap is hard-coded and ignores `--fanout-cap 0` (graphql.md t5, asyncapi.md fan). That differs from the documented "never guess" meaning for every other language.
    - **M.**

14. **Recorded includes that are never emitted.**
    - The Verilog `` `include `` arm exists (cross_file.rs:2123-2140, `lang == "verilog"`), but verilog.rs never pushes an ImportRecord.
    - Fortran `include` is likewise unreachable (fortran.md §04).
    - VHDL is listed in that arm but emits only `library`/`use`.
    - Kivy `#:include` and asm `.include` are ignored.
    - **S** each, by emitting the record.

15. **Single-language parse defects in import forms**, each **S**:
    - C# `global using static` (csharp.rs:428, heads.rs:875; fixture f10).
    - Groovy `import c.*` losing `*` (groovy.md f5).
    - Python comma imports defeating framework detection (python.md f12).
    - PowerShell backslash paths (powershell.md f2, f4).
    - Go external dot-import opening any-directory binding (cross_file.rs:4154-4158; go.md f14).
    - Lean current namespace not auto-opened (lean.md `imp/ns_same`).
    - Elixir imports file-scoped instead of module-scoped (elixir.md f5).
    - Fortran `use` file-wide instead of per program unit (fortran.md b).

16. **Dynamic and computed imports are unmodelled everywhere.**
    - JS `import()`/`require(var)`, Python `importlib`, Bash `$VAR` source, CMake `${}`, R `source(file.path(…))`, PHP `__DIR__ .` (basename-only), Perl `require $mod` and PowerShell `$PSScriptRoot` are all affected.
    - No language resolves them. Python, JS, Java, C#, Ruby and PHP only screen them for dead code (`dynuse`, §15).
    - Constant-folding of `$PSScriptRoot`/`__DIR__`/`${CMAKE_CURRENT_LIST_DIR}`/`$(dirname "$0")` would be **M**.

## Doc corrections

1. **ruby.md §04 says "with transitive depth 4".** That is wrong. The `"require"` arm passes depth 4 (cross_file.rs:1867-1876), but `collect_include_defs` recurses only into imports of kind `include` or `system-include` (cross_file.rs:3507-3510). A required file's own `require`s are not followed, and the same applies to Clojure `require`, Lua legacy reach and Perl `require`.
2. **ruby.md §04 calls the wrong-file high edge "unverified" and gives a likely cause.** The cause is now verified by fixture `wc`/`wc2`. `direct_imports` holds `simple → [qualified_name]`, and the step-1 lookup goes through `lookup_with_reexports(lang, qn, by_qn, …)` (cross_file.rs:5710-5755). That keeps one callable per bare QN, the last in path order, so the binding flips when the decoy directory is renamed from `zz/` to `aa/`. It is the same defect that bash.md, starlark.md and cmake.md document.
3. **powershell.md §04 says "f3: `. ./lib/helper.ps1` → … `Main -> Shared high` (lib copy only, although `other/dup.ps1` also defines `Shared`)".** That result depends on path order, not on the dot-source. In fixture `wc` (decoy `zz/h2.ps1`), `Main -> Get-Dup` bound **`zz/h2.ps1` at high**. With `aa/` it bound `lib/h.ps1`. The doc should list PowerShell under the wrong-copy bug.
4. **r.md §04 says "f2: `source("R/util.R")` makes `util_fn` bind at high".** That is true only when no same-named copy sorts after it. In fixture `wc`, `source("lib/u.R")` bound `mainr -> dupr@zz/u2.R` at high.
5. **solidity.md §04 says "Fixture f6: `afn()` … binds `high`".** That is true only without a decoy. In fixture `wc`, `import "./lib/A.sol"` bound `C.m -> dsol@zz/B.sol` at high. Solidity should be listed with the wrong-copy defect.
6. **java.md §04 says modules.rs covers "JS/TS/Python/Go/Erlang only", and csharp.md §04 says "JS/TS/Python/Go/Erlang/Lua".** `modules.rs:189-191` `indexed()` is exactly `javascript | typescript | python | go | lua`. Erlang `m:f` is handled in `cross_file.rs:5566-5602`, not in `ModuleIndex`, and produces no `not-in-imported-module`. CLAUDE.md's list ("… Go packages, Erlang `m:f`") has the same inaccuracy. This matrix does not edit CLAUDE.md.
7. **Many docs describe heads.rs as covering "Python/JS/TS/Go/Java/Kotlin/C#/C++"**, citing CLAUDE.md. The affected docs are bash.md, hcl.md, julia.md, clojure.md, lean.md, nix.md, starlark.md, cmake.md, ocaml.md and r.md. The actual `external_head` list also includes **Scala** (heads.rs:668). The conclusions those docs draw are unaffected.
8. **fsharp.md §04 says "The `match lang` arms are Python, JS/TS, Go, Java/Kotlin, C#, Scala, Solidity and C++ (heads.rs:413-625)".** Those are the import-*binding* arms. Solidity appears there only for `using-for` (heads.rs:617-624). `external_head` excludes Solidity (heads.rs:660-671), so Solidity has no external refusal, as solidity.md correctly states.
9. **lua.md §04 says "`modules.rs:183-186, 253-265`… Not-in-imported-module refusal … in fixture p1 `util.missing()` lands in the external bucket".** The code path (`modules.rs:1692-1694` → `Scoped::Refused`) and the observed audit disagree. The likely reason is the general rule that the refusal is recorded only when another callable carries the name (cross_file.rs:2810-2827), which python.md, go.md and elixir.md document. That is unverified for Lua.

## Fixtures run for this matrix

All of these are under `<scratch>/parity/matrix-04/`, run with `target/release/cgg . -t json -o out.json`. Edges were printed with `edges.py` in the same directory.

- **`wc/`**:
  - Ruby: `main.rb` does `require_relative 'lib/one'` and calls `dup_fn`.
  - R: `main.R` does `source("lib/u.R")` and calls `dupr`.
  - PowerShell: `main.ps1` does `. ./lib/h.ps1` and calls `Get-Dup`.
  - Solidity: `C.sol` does `import "./lib/A.sol"` and calls `dsol`.
  - Each language has a decoy copy under `zz/`. Every edge went to `zz/…` at high, `cross-file:imports`.
- **`wc2/`**: the same tree with `zz/` renamed to `aa/`. Every edge went to `lib/…` at high.
