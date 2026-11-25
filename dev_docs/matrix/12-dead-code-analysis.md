# 12. Dead-code analysis

> **Re-checked against 0.13.3 (2026-11-25).** Written against 0.11.0. Re-verified at 0.13.3: the declared `PluginSignals` per plugin (unchanged except Kotlin `value_refs`, now declared); the **ValRef** column with a function-passed-as-value fixture and a control per language (`scratchpad/matrix-docs/m12/vr`, `vr/ctl`); **ExtContract** for Kotlin, Swift, Scala, TypeScript, Dart, C++, PHP and Python async (`m12/ov`); **Unreach** (`m12/un`); the Kotlin ceiling (`m12/kh`); and the CMake, Erlang, Clojure, Fortran and Elixir notes (`m12/misc`). Other cells and the per-cell evidence section are the 0.11.0 reading; changes are marked with the release that made them.

This category covers `--dead-code` (and `--why-live` / `--report-unreferenced`, which share its engine in `crates/cgg-resolve/src/deadcode/`). It asks how well cgg can say "nothing calls this" for each language, and how honest it is about that claim. The answer depends on five things:

1. **The language class** (`caps::measure`, `crates/cgg-resolve/src/deadcode/caps.rs:163-172`):
   - `descriptor`: every finding is withheld (`DESCRIPTOR`, caps.rs:41).
   - `script-driven`: findings are capped at `low` and root-dependent ones are withheld (`SCRIPT_DRIVEN`, caps.rs:27-36).
   - `analyzable`: the language has visibility *and* attributes, declared or observed.
   - `degraded`: everything else.
2. **The signals extracted** (`PluginSignals`, `crates/cgg-lang/src/lib.rs:269-288`): visibility, attributes, exports, test roles, value references, dynamic uses, unreachable regions and impls. The capability table *declares* them, and *observation* can only upgrade one (caps.rs:96-107, 124-152).
3. **The roots discovered** (`roots::discover`, `crates/cgg-resolve/src/deadcode/roots.rs`): program entries, lifecycle/runtime hooks, FFI exports, declared exports, top-level invocations, macro invocations, framework entries/marks, test roots and user roots.
4. **Confidence caps** (`Evidence::cap`, `crates/cgg-core/src/deadcode.rs:298-357`):
   - `implements-external-contract`, `macro-generated` and `implemented-trait-surface`;
   - reflective `name-collides-with-screened-site` evidence (screens `reflection`, `value-position`, `attribute-string`);
   - `public-visibility`, `implicitly-invokable-kind`;
   - the four `language-lacks-*` caps.
5. **Unresolved-site doubt** (`evidence.rs:170-200`, `mod.rs:655-680`): a same-named unresolved call site lowers a finding. For Java and C# only, doubt is filtered by whether the site could legally reach the member.

A finding reaches `high` only when the class is `analyzable` (`signals_complete`, caps.rs:88-96) **and** it carries none of the `language-lacks-value-references` / `language-lacks-dispatch-model` caps (mod.rs:913-924).

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | full: the capability works for the language's mainstream forms |
| 🟡 | partial: works for some forms, works only by observation or by accident, is extracted but not consumed, or has a verified defect |
| ⬜ | none: the language has the concept and cgg does not support it |
| — | n/a: the concept does not exist in the language, or the language's class makes it moot (descriptor languages withhold every finding) |

### Columns

| Column | What it measures | Source |
| --- | --- | --- |
| **Class** | Language class and the confidence a finding can actually reach. `A`: analyzable, `D`: degraded, `S`: script-driven, `X`: descriptor. The value after the dot is the *effective* ceiling. An asterisk means the capability JSON reports `max_confidence: high` but every finding is still capped at medium. | caps.rs:163-172, mod.rs:1149 |
| **High** | A finding can reach `high`. | caps.rs:88-96, mod.rs:913-924 |
| **Vis** | The visibility signal: `private`/`internal` corroborates (`private-visibility`), and `public`/`protected` caps at medium (`public-visibility`). | mod.rs:945-970 |
| **Attrs** | Real source attributes, decorators or annotations are recorded. They feed framework, test and FFI roots and override markers. | caps.rs:124-152 |
| **ValRef** | A function passed or stored as a value is live. This means `Via::Reference` edges, or `RefRecord::value_ref` sites. | caps.rs:140-150 |
| **Dispatch** | An override or implementation reached through a base, interface or trait declaration is live. This covers `dispatch::inheritance_fanout` under `--dead-code`, Rust trait impls, and C++ `virtual`. | dispatch.rs:110-123 |
| **Export** | Declared exports or public API are roots (`lang:export`, `rust:reexport`, or a framework rule). 🟡 means public symbols are only capped at medium, not rooted. | roots.rs:890-938 |
| **Entry** | The language's program-entry form is a root: `UNIVERSAL_ENTRY` `main`, plus `LANG_ENTRY`. | roots.rs:20-32 |
| **Hooks** | Runtime-invoked hooks are roots: `LIFECYCLE`, `CONVENTIONAL_METHODS`, dunders, `IMPLICIT_RUST_TRAITS`, and KV bindings. | roots.rs:37-171, 725-737, 844-886 |
| **FFI** | Symbols exported to another language or process are roots (`ffi:export`). | roots.rs:173-183, 760-778; ffi.rs:643-677 |
| **TopLvl** | A call with no enclosing callable roots its target (`toplevel:invocation`). The match is by `(language, simple_name)` everywhere, so the rule can over-root in every language. 🟡 marks documented *misses*. | roots.rs:552-614 |
| **Macro** | Macro-generated code: the `toplevel:macro-invocation` root plus the `macro-generated` cap. | roots.rs:616-705, mod.rs:893-897 |
| **ExtContract** | The `implements-external-contract` cap, for an override of a base or interface declared outside the tree. | mod.rs:470-531 |
| **Reflect** | The reflective-literal screen caps same-named findings at medium. | dynuse.rs:26-56, mod.rs:614-638 |
| **Doubt** | A same-named unresolved site lowers a finding (`name-matches-unresolved-site` / `ambiguous-site-in-same-file`). | evidence.rs:102-200 |
| **VisDoubt** | Doubt is filtered by whether the unresolved site could legally reach the member (the `can_reach` closure). | mod.rs:655-680 |
| **Tests** | `TestRole` tagging, which enables the `only-used-by-tests` category. | lib.rs:277, mod.rs:820-826 |
| **Unreach** | Statements after an unconditional terminator. At 0.11.0 **no language consumed the regions** (see gap 2), so the best mark was 🟡 (extracted only). Since 0.11.1 the eight plugins that compute them (c, cpp, go, java, javascript, typescript, python, rust) feed `cgg_resolve::unreachable`, giving `incoming-only-from-unreachable-code` (fixtures `m12/un/{py,go}`). | cfg.rs; cgg/src/lib.rs:1935 |
| **Blind** | The report names the language's known blind spots (`BLIND_SPOTS`). | caps.rs:44-68 |

## The matrix

| Language | Class | High | Vis | Attrs | ValRef | Dispatch | Export | Entry | Hooks | FFI | TopLvl | Macro | ExtContract | Reflect | Doubt | VisDoubt | Tests | Unreach | Blind | Notes (most important gap) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| rust | A·high | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — | ✅ | ⬜ | ✅ | ✅ (0.11.1) | ✅ | `#[cfg(test)]` helpers are test code since 0.11.1 (matrix 11 fixture `rs`). Trait-declaration methods are false positives. `CONVENTIONAL_METHODS` over-roots (a trait default `describe` is rooted). Doubt is not filtered by `pub(crate)`/private reach. |
| python | A·high | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — | ✅ | — | ✅ (async fixed 0.11.1) | ✅ | ✅ | ⬜ | ✅ | ✅ (0.11.1) | ✅ | Fixed in 0.11.1: an `async def` method of a class with an outside base now carries `implements-external-contract` (fixture `m12/ov/python`: `_ahook` medium). |
| kivy | D·med | ⬜ | — | 🟡 | ⬜ | — | — | — | ✅ | — | ✅ | — | — | — | 🟡 | — | — | — | ✅ | Unresolved KV sites (`self.parent.save()`) are not doubt for the Python methods they name, because the index is keyed by `(language, name)`. |
| javascript | D·med | ⬜ | 🟡 | ✅ | 🟡 | ✅ | ⬜ | ✅ | 🟡 | — | ✅ | — | ⬜ | 🟡 | ✅ | ⬜ | ⬜ | ✅ (0.11.1) | ✅ | `visibility` is not declared, so every finding is capped at medium. `export` is never a root. |
| typescript | D·med | ⬜ | 🟡 | ✅ | 🟡 | ✅ | ⬜ | ✅ | 🟡 | — | ✅ | — | ✅ `override` recorded (0.11.1) | 🟡 | ✅ | ⬜ | ⬜ | ✅ (0.11.1) | ✅ | `export` is not a root. `override` caps at medium via `implements-external-contract` since 0.11.1 (fixture `m12/ov/typescript`). |
| go | D·med | ⬜ | ✅ | — | 🟡 | ⬜ | 🟡 | ✅ | 🟡 | ⬜ | ✅ | — | ⬜ | ✅ | ✅ | ⬜ | ✅ | ✅ (0.11.1) | ⬜ | Go has no attributes, so the `attributes` requirement makes it `degraded` forever. It also declares no `impls`, which adds the dispatch cap. Exported names are capped, not rooted. |
| java | A·high | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | — | ✅ | — | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ (0.11.1) | ⬜ | Extraction gaps make code reached only from `static {}` blocks, field initializers, `this(…)`/`super(…)` or `new Generic<>()` false-dead. |
| kotlin | A·med* | ⬜ | ✅ | ✅ | ✅ (0.11.1) | 🟡 | 🟡 | ✅ | 🟡 | — | ✅ | — | ✅ `override` recorded (0.11.1) | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | `value_refs` declared since 0.11.1 (`::f` references), but still no `impls`, so `language-lacks-dispatch-model` caps every finding at medium although the report says `high` (fixture `m12/kh`, 0.13.3). `override` is recorded since 0.11.1. |
| c | D·med | ⬜ | ⬜ | 🟡 | ✅ (0.11.1) | — | ⬜ | ✅ | 🟡 | ⬜ | ✅ | ✅ | — | ⬜ | ✅ | ⬜ | ⬜ | ✅ (0.11.1) | ⬜ | `static` vs external linkage is not distinguished. Verified: f3 `s_dead` medium like any extern. `__attribute__((constructor))` and function-pointer initialisers are false-dead. |
| cpp | D·med | ⬜ | ⬜ | ✅ | ✅ (0.11.1) | 🟡 | ⬜ | ✅ | 🟡 | ⬜ | ✅ | ✅ | ✅ `override` recorded (0.11.1) | — | ✅ | ⬜ | ⬜ | ✅ (0.11.1) | ⬜ | No visibility, which caps every finding. `extern "C"` is not an FFI root. Verified: f3 `c_export` is a finding. `override` is recorded since 0.11.1. |
| csharp | A·high | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ | ✅ | — | 🟡 | ✅ | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | An implicit interface implementation gets no contract cap. The contract is labelled with the first outside ancestor, not the right one. Constructors are never reached. |
| bash | S·low | ⬜ | — | — | ⬜ | — | — | ✅ | ⬜ | — | ✅ | — | — | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | Everything is withheld. `trap handler EXIT` and `"cmd_$1"` dispatch are invisible, and there is no blind-spot disclosure for either. |
| ruby | D·med | ⬜ | ⬜ | — | 🟡 | 🟡 | ⬜ | ✅ | 🟡 | — | 🟡 | — | ⬜ | ✅ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | No `private` section tracking, so private and public look the same. A top-level paren-less call does not root its target. |
| swift | D·med | ⬜ | ⬜ | ⬜ | ✅ (0.11.1) | 🟡 | ⬜ | 🟡 | 🟡 | — | ✅ | ⬜ | ✅ `override` recorded (0.11.1) | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | `@objc`/`@IBAction`/`@main`, `Codable` hooks and operators are not roots (attributes still not captured at 0.13.3). `override` (0.11.1) and `#selector` values (0.11.1) are read. |
| lua | D·med | ⬜ | ⬜ | — | ✅ (0.11.1) | ⬜ | ⬜ | ✅ | ⬜ | — | 🟡 | — | — | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | Metamethods (`__index`, `__call`) are never rooted: `is_dunder` needs a trailing `__`. A returned module table is not an export. |
| php | D·med | ⬜ | ⬜ | ✅ | 🟡 | ✅ | ⬜ | ✅ | ⬜ | — | ✅ | — | ✅ `override` recorded (0.11.1) | ✅ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | Visibility is never extracted, so nothing reaches `high`. Magic methods (`__construct`, `__invoke`) are not roots, though `new T` reaches `__construct` since 0.11.1. `#[\Override]` matches since 0.11.1. |
| dart | D·med | ⬜ | ⬜ | ⬜ | ✅ (0.11.1) | 🟡 | ⬜ | ✅ | ⬜ | — | ✅ | — | ✅ `override` recorded (0.11.1) | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | `@override` is recorded since 0.11.1 (fixture `m12/ov/dart`: `W::build` carries `implements-external-contract`). There are no Flutter lifecycle roots and `_private` is not read. |
| scala | D·med | ⬜ | ⬜ | ⬜ | ✅ (0.11.1) | 🟡 | ⬜ | ✅ | 🟡 | — | ✅ | ⬜ | ✅ `override` recorded (0.11.1) | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | No visibility or annotations are recorded. `override def` of an outside base is capped since 0.11.1 (fixture `m12/ov/scala`). `apply`/`unapply`/`given` are not roots. |
| hcl | S·low | ⬜ | — | — | — | — | ⬜ | — | — | — | 🟡 | — | — | — | 🟡 | — | — | — | ⬜ | References (`var.x`, `module.y`) are not recorded, so every block is "unreferenced". Withholding everything is the only honest output. |
| zig | D·med | ⬜ | ⬜ | — | ✅ (0.11.1) | — | ⬜ | ✅ | — | ⬜ | ✅ | — | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | `pub` is not recorded. `export fn` / `callconv(.c)` are not FFI roots. Sites in the external bucket give no doubt. |
| objc | D·med | ⬜ | — | ⬜ | ✅ `@selector`, C function values (0.11.1) | ⬜ | ⬜ | ✅ | 🟡 | — | ✅ | ⬜ | ⬜ | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | No signals at all. `@selector`/`IBAction` targets and C calls inside methods are invisible, which causes many false findings. |
| r | S·low | ⬜ | — | — | ⬜ | ⬜ | ⬜ | ✅ | 🟡 | — | 🟡 | — | — | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | A top-level call to a `source()`d file's function is not a root (reason `no-candidate-in-file`). `NAMESPACE`/`@export` are not read. |
| groovy | D·med | ⬜ | ⬜ | 🟡 | 🟡 `this.&m`/`Type.&m` (0.11.1); `obj.&m` through an untyped variable binds nothing (fixture `m12/vr/groovy`) | 🟡 | ⬜ | ✅ | 🟡 | — | ✅ | — | ✅ | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | No visibility. No reflection screen (`invokeMethod`, `this."name"()`). Paren-less calls leave no doubt site. |
| julia | D·med | ⬜ | — | ⬜ | ⬜ | 🟡 | ⬜ | ✅ | 🟡 | — | ✅ | ⬜ | 🟡 | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | `export` is not read. Functions passed as values (`map(f, v)`) and calls reached through `using` are false-dead. |
| perl | S·low | ⬜ | — | — | 🟡 | ⬜ | ⬜ | ✅ | ⬜ | — | 🟡 | — | ⬜ | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | `@EXPORT` is not read. `DESTROY`/`AUTOLOAD`/`BEGIN`/`import` are not roots. A qualified top-level call does not root. |
| elixir | D·med | ⬜ | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | ✅ | ✅ | — | ✅ | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | `def` vs `defp` is not recorded (verified: f2 `pub_f` and `priv_f` get identical evidence). Later clauses of a called function are false-dead. |
| erlang | D·med | ⬜ | — | ⬜ | ✅ (0.11.1) | ⬜ | ⬜ | ✅ | ✅ | — | ✅ | ⬜ | ⬜ | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | `-export([...])` is not a root, which is the largest false-positive source for libraries (still true at 0.13.3, fixture `m12/misc/erlang`). `fun f/1` and MFA targets are value references since 0.11.1. |
| fortran | D·med | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | 🟡 | — | ⬜ | — | — | — | — | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | `PROGRAM` is not a root, so the usual result is `no-roots-found`. `bind(c)` and `public ::` are not read. Name matching is case-sensitive. |
| clojure | D·med | ⬜ | ⬜ | ⬜ | ✅ (0.11.1) | ⬜ | ⬜ | 🟡 | ⬜ | — | ✅ | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | `-main` is not a root. `defn-`/`^:private`/`^:export` are not read. Protocol implementations are false-dead. |
| haskell | D·med | ⬜ | — | — | ✅ (0.11.1) | ⬜ | ⬜ | ✅ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | 🟡 | — | ⬜ | — | ⬜ | The module export list is ignored. Qualified, operator and composition uses are invisible and leave no doubt site. |
| ocaml | D·med | ⬜ | — | ⬜ | ✅ (0.11.1) | — | ⬜ | ✅ | — | — | ✅ | ⬜ | — | — | ✅ | — | ⬜ | — | ⬜ | `.mli` signatures are not read, so a library's `val` API reads as dead. Values passed to `List.iter` and through `\|>` are value references since 0.11.1 (fixture `m12/vr/ocaml`). |
| powershell | S·low | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | 🟡 | — | ✅ | — | — | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | `Export-ModuleMember` / `FunctionsToExport` are not roots. Doubt matching is case-sensitive although the language is case-insensitive. |
| solidity | D/A·med* | ⬜ | 🟡 | 🟡 | ⬜ | ⬜ | ✅ | — | ⬜ | — | ✅ | — | ⬜ | — | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | Modifiers and constructors are false-dead. `internal` is treated as private (inheritable). `override` of an OpenZeppelin base is not capped. |
| fsharp | D·med | ⬜ | ⬜ | 🟡 | ✅ (0.11.1) | ⬜ | ⬜ | 🟡 | — | — | ✅ | — | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | ⬜ | ⬜ | `[<EntryPoint>]` with a name other than `main` is reported dead. `private`/`internal` are not read. Functions used through pipes or as values are value references since 0.11.1 (fixture `m12/vr/fsharp`). |
| starlark | S·low | ⬜ | ⬜ | — | ⬜ | — | ⬜ | ✅ | ⬜ | — | 🟡 | ⬜ | — | — | 🟡 | ⬜ | — | — | ⬜ | A BUILD-file call of a `load()`ed macro does not root it. `implementation = _impl` is invisible. |
| cmake | S·low | ⬜ | — | — | ⬜ | — | — | ✅ | — | — | 🟡 | — | — | ⬜ | ✅ | — | ⬜ | — | ⬜ | `CMakeLists.txt` is analyzed since 0.11.1 (detected by name), but its top-level call of an `include()`d file's function is `no-candidate-in-file` and roots nothing (fixture `m12/misc/cmake`, 0.13.3). Top-level matching is case-sensitive. |
| nix | S·low | ⬜ | — | — | ⬜ | — | ⬜ | ✅ | — | — | ✅ | — | — | — | ✅ | — | — | — | ⬜ | Attrset members a file returns (its de-facto exports) are not roots. Value uses (`map helper`) are invisible. |
| verilog | D·med | ⬜ | — | 🟡 | — | — | — | ⬜ | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | — | ⬜ | **Bug:** every `task` is rooted as a framework callback, because its kind tag `task` is in `FRAMEWORK_ATTRS`. The top module is not a root, and called functions are reported dead. |
| vhdl | D·med | ⬜ | — | 🟡 | — | — | — | ⬜ | — | — | ⬜ | — | — | — | ⬜ | — | ⬜ | — | ⬜ | No roots at all, and no call sites, so nearly every subprogram is reported. |
| asm | D·med | ⬜ | ⬜ | 🟡 | ⬜ | — | ⬜ | ✅ | — | ⬜ | ✅ | ⬜ | — | — | ✅ | — | — | — | ⬜ | `.globl`/`global` symbols are not export or FFI roots. Local labels, fall-through and address-taken labels give structural false positives. |
| smithy | X·withheld | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | — | Withheld by design. `--report-unreferenced` still lists nodes (unverified for smithy; verified for graphql). |
| proto | X·withheld | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | — | Withheld by design. No roots are produced. |
| graphql | X·withheld | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | — | `--report-unreferenced` does not withhold descriptor languages, so `Query`/`Mutation` roots are listed as unreferenced. |
| openapi | X·withheld | — | — | — | — | — | — | — | — | — | ✅ | — | — | — | — | — | — | — | — | Withheld by design. A top-level `$ref` is the only root. |
| asyncapi | X·withheld | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | — | — | Withheld by design. |
| lean | D·med | ⬜ | 🟡 | 🟡 | ⬜ | ⬜ | ⬜ | ✅ | ⬜ | — | ✅ | ⬜ | ⬜ | — | ✅ | ⬜ | ⬜ | ⬜ | ⬜ | Visibility capture never works: it scans the wrong node. `@[simp]` and `instance` are captured but no root rule reads them, so public theorems are reported dead. |

### Per-cell evidence (non-obvious cells)

**Class, High**

- Only Rust, Python, Java and C# can produce `high`. They are the only plugins that declare visibility, attributes, `value_refs` and `impls`:
  - rust.rs:38-49
  - python.rs:44-56
  - java.rs:18-28
  - csharp.rs:42-51
- Every other language carries at least one Medium-capping `language-lacks-*` item (deadcode.rs:323-330).
- Verified on fixture `f1` (Java, Python, Kotlin, Go, TS, PHP, Swift): only `p.A.priv` (Java) and `a._priv` / `a.P._ahook` (Python) were `high`.

**Kotlin `A·med*`**

- The capability JSON reports `kotlin analyzable … max_confidence high` (fixture f1).
- Yet `p.K.priv` (private, unreferenced) was `medium` with `language-lacks-dispatch-model` and `language-lacks-value-references`. It carried `language-signals-complete` at the same time.
- `max_confidence` is computed from `signals_complete()` alone (mod.rs:1149), which ignores the two caps.

**Solidity `D/A·med*`**

- The class is `analyzable` only when a `library-member` attribute is observed (solidity docs §12).
- Solidity declares no `value_refs` or `impls` (solidity.rs:21-26), so findings stay medium either way.

**Vis 🟡**

- JS/TS record `Vis` for class members (typescript.rs:256-270 tests) but do not declare `visibility`, so `language-lacks-visibility` is still pushed.
- Verified: fixture f1 `X.p` carries both `private-visibility` and `language-lacks-visibility`.
- Solidity treats `internal` as private (mod.rs:951-953).
- Lean's capture never fires (lean.md §12).

**Attrs**

- Declared: rust, python, java, kotlin, csharp, cpp, javascript, typescript, php.
- 🟡 means upgraded only by observation from non-attribute tags (caps.rs:124-136):
  - C: the `macro` tag;
  - Verilog/VHDL/asm: kind tags;
  - Solidity: `library-member`;
  - F# and Groovy: real annotations, but undeclared;
  - Lean: `@[simp]`/`instance`;
  - Kivy.
- Swift and Scala show `attributes: full` only when an `abstract` tag exists (swift.md, scala.md §12). Fixture f1/f2 showed `none` for both, so they are marked ⬜.

**ValRef**

- Declared by: rust, python, java, csharp, javascript, typescript, go, php, ruby, perl, elixir; kotlin too since 0.11.1.
- Since 0.11.1 (WS26) value references are recorded under `--dead-code` for many more languages whether or not the signal is declared. Fixture `m12/vr` plus a control with the use removed (`m12/vr/ctl`) shows the reference keeping `cb` live in c, cpp, clojure, dart, erlang, fsharp, haskell, kotlin, lua, ocaml, scala, swift and zig; julia (`map(cb, …)`) is still a finding; groovy binds only `this.&m`/`Type.&m`.
- The 🟡 notes below are the 0.11.0 reading:
- 🟡 where the docs show that only registrar arguments are seen:
  - TS §12 (`mappedFn`, `timerFn`);
  - Go §12 (`lib.less`);
  - C/C++ (struct-initialiser and pointer-table false positives);
  - Ruby (symbol arguments only);
  - Perl (`\&name`);
  - Elixir (`&f/1`).

**Dispatch**

- ✅ means in `INHERITANCE_LANGS` (dispatch.rs:110-123) **and** declaring `impls`: python, java, csharp, typescript, javascript, php. Rust is ✅ through trait impls.
- 🟡 means one of the following:
  - inheritance fan-out runs but the `language-lacks-dispatch-model` cap still applies, because `impls` is undeclared (kotlin, scala, groovy, swift, dart);
  - `impls` is declared but never produced (ruby);
  - only C++ `virtual` edges reach overrides, with no `implemented-trait-surface` evidence because cpp is not in `INHERITANCE_LANGS` (cpp.md §12);
  - Julia, where `julia-base-dispatch` (rules.rs:6421) roots methods extending `Base` by rule.

**Export**

- Python: `__all__` (python.rs:1208-1247).
- Rust: `pub use` (rust.rs:136, roots.rs:917-938).
- These are the only `ExportRecord` writers (grep of `crates/cgg-lang/src`).
- Solidity is ✅ through the `solidity-public` framework rule (rules.rs:1419).
- 🟡 means public symbols are only capped through `public-visibility`: go, java, kotlin, csharp.

**Entry**

- `LANG_ENTRY` (roots.rs:23-32) covers go `init`/`TestMain`, elixir/erlang `start`/`start_link`/`init`, c/cpp `_start`/`WinMain`/`DllMain`, zig/asm/rust `_start`. Everything else gets `main` only.
- 🟡 Swift: `@main` types are not rooted.
- 🟡 Fortran: `PROGRAM`.
- 🟡 Clojure: `-main`.
- 🟡 F#: `[<EntryPoint>]` with another name.
- ⬜ Verilog/VHDL: the top-level module or entity.
- For script and markup languages the entry is "top-level code", which is rooted through TopLvl. They are marked ✅.

**Hooks**

- ✅ means a language-specific table or rule covers the mainstream runtime hooks:
  - Rust `IMPLICIT_RUST_TRAITS`;
  - Python dunders;
  - Java/C# `LIFECYCLE`;
  - Elixir/Erlang OTP;
  - Kivy `kivy:kv-binding` (roots.rs:733).
- 🟡 means `CONVENTIONAL_METHODS` only, or a short `LIFECYCLE` row with documented misses:
  - Kotlin operators;
  - Swift (three UIKit names);
  - Go `String()`;
  - Ruby `method_missing`, `each`;
  - Scala `apply`/`unapply`;
  - C `__attribute__((constructor))`;
  - JS/TS getters (capped only);
  - PowerShell DSC through a framework rule;
  - R `.onLoad` through `r-package-hooks` (rules.rs:6210).
- ⬜ PHP magic methods and Lua metamethods: `is_dunder` needs `__x__` (roots.rs:967-969). Verified: f1 `C::__construct` is a finding.

**FFI**

- `classify_ffi` (ffi.rs:643-677) plus `FFI_EXPORT_ATTRS` (roots.rs:173-183).
- Rust ✅.
- C# ✅: `[UnmanagedCallersOnly]` matches `unmanagedcallersonly` in `classify_ffi`. Verified: fixture f3 root `N.E.Exp ffi:export exported over c-abi`.
- C/C++ ⬜: no plugin emits the `extern:c` key that `classify_ffi` matches (grep of `crates/cgg-lang/src/plugins` is empty). Verified: f3 `c_export` is a finding.
- ⬜ also for Zig `export fn`, Fortran `bind(c)`, Haskell `foreign export`, asm `.globl` and Go cgo `//export` (from the language docs; Go is unverified).

**Macro**

- The `macro:` tag comes from `rust_macro.rs` and `c_macro.rs`.
- The `toplevel:macro-invocation` root is C-family (roots.rs:616-660) and Rust (roots.rs:665-705).
- ⬜ is given to languages with a macro or codegen system whose output is not tagged: Elixir `defmacro`, Clojure, Julia `@macro`, Lean, Scala macros, Swift macros, OCaml ppx, Haskell TH, Erlang `-define`, ObjC (C preprocessor), Verilog `` `define ``, Starlark macros, asm macros.

**ExtContract** (mod.rs:470-531)

- Rust: the impl's trait is not declared in the tree.
- C# `partial` parts (`PARTIAL_DECLARATION`).
- A marker in `OVERRIDE_MARKERS` (`@Override`, `override`, `@override`, mod.rs:409). Only java.rs, csharp.rs and groovy.rs (`attrs::collect_with_preceding`, groovy.rs:192) put such a marker into `attributes`.
- Python, unmarked, for non-`Function` kinds only (mod.rs:505), which is why its `async def` methods missed the cap at 0.11.0. Fixed in 0.11.1: a definition is a method when its owner is a class (fixture `m12/ov/python`).
- Verified ✅: Groovy (f2 `G.hook implements-external-contract:Base`).
- Verified ⬜ at 0.11.0: Scala, Dart, Ruby (f2), Kotlin, TS, Swift, PHP (f1). Since 0.11.1 Kotlin, Swift, Scala, TS, Dart, C++ and PHP record the marker (fixtures `m12/ov/*`); Ruby was not re-run.
- Julia 🟡: `julia-base-dispatch` roots `Base` extensions instead of capping them.

**Reflect**

- `REFLECTIVE` (dynuse.rs:26-56) covers python, ruby, java, csharp, javascript, typescript, php and go.
- JS/TS list only `require`/`importScripts`, which are `DYN_MODULE_LOADERS` (facts.rs:165) → screen `module-load`, which **does not cap** (mod.rs:620-638). They are therefore 🟡: the doubt is recorded but there is no reflection cap.
- Go ✅: `MethodByName`/`FieldByName`. It is extracted (go.rs:115) although `dyn_uses` is not declared (go.rs:40-48).

**Doubt**

- Generic everywhere.
- 🟡 where the docs show a defect:
  - Kivy: cross-language sites;
  - Zig, Dart: external-bucket or refused sites do not count;
  - Fortran, PowerShell: case-sensitive;
  - Haskell, F#, Groovy, HCL, Starlark: the sites are never recorded.
- ⬜ Verilog/VHDL: no call sites exist.

**VisDoubt**

- `can_reach` (mod.rs:655-680) has arms for Java `private` (the file), Java package-private (the package) and C# `private` (the file or same-named owner) only.
- C# `internal` and every other language fall through to `true`.

**Tests**

- `test_defs` is declared by rust (rust.rs:44), python (python.rs:50) and go (go.rs:42) only. Those are also the only plugins that write `TestRole` (grep).

**Unreach**

- `unreachable_after_terminator` is called by rust.rs:120, python.rs:119, go.rs:111, java.rs:53, javascript.rs:76, typescript.rs:84, c.rs:59 and cpp.rs:71.
- At 0.11.0 the only other mention was the capability-bit copy at `crates/cgg/src/lib.rs:1935`, and the doc fixtures (Python f8, JS f9, TS f6, Go f5, C f4) all still drew the edge after `return`.
- Since 0.11.1 `crates/cgg-resolve/src/unreachable.rs` consumes the regions under `--dead-code`: edges from them are capped at `medium` and the liveness adjacency skips them (fixtures `m12/un/{py,go}`).

**Blind**

- `BLIND_SPOTS` (caps.rs:44-68) has rows for rust, python, javascript, typescript and kivy only.

## Parity gaps

Ranked by how much wrong output they cause, weighted by how common the language is. Effort: S is under a day and a table or one-file change; M is a plugin change plus tests; L is a new model.

1. **Exports and public API are almost never roots** (M per language). Only Python `__all__` and Rust `pub use` write `ExportRecord`s (python.rs:1208-1247, rust.rs:136). The following are all ignored, so library code with no in-tree caller is reported dead:
   - JS/TS `export`;
   - Erlang `-export`;
   - Haskell export lists;
   - OCaml `.mli`;
   - Elixir `def`;
   - Julia `export`;
   - R `NAMESPACE`;
   - PowerShell `Export-ModuleMember`;
   - Perl `@EXPORT`;
   - Fortran `public ::`;
   - Swift `public`/`open`;
   - Dart non-`_` names under `lib/`;
   - Lua returned tables.

   The JS/TS plugin already parses `export` statements for import resolution (`ModuleIndex` re-exports, CLAUDE.md step 3). Evidence: typescript.md §12 (`exportedButUnused`), erlang.md §12 (fixture f1), haskell.md §12 (fixture f3), and this matrix's fixture f1 `exported`.

2. *Fixed in 0.11.1 (fixtures `m12/un/{py,go}`: `incoming-only-from-unreachable-code`).* **Unreachable regions are extracted for 8 languages and consumed by none** (S–M). A call after `return`/`throw` still keeps its target live.
   - Plugin call sites: rust.rs:120, python.rs:119, go.rs:111, java.rs:53, javascript.rs:76, typescript.rs:84, c.rs:59, cpp.rs:71.
   - No reader exists outside `crates/cgg/src/lib.rs:1935`.
   - The effort is a filter in `Adj::build`, or a resolver pass that drops sites inside `FileFacts::unreachable`.
   - The other statement languages (C#, Kotlin, Swift, Dart, Scala, PHP, Ruby) do not even extract the regions (no `unreachable` in csharp.rs/kotlin.rs signals, signals listed above).

3. *Fixed in 0.11.1 (fixtures `m12/ov/*`: Kotlin, Swift, Scala, TS, Dart, C++ and PHP overrides of an outside base carry `implements-external-contract`).* **`override` markers are captured for Java, C# and Groovy only** (S each). Kotlin, Swift, Scala and TS write an `override` modifier, Dart writes `@override`, and PHP writes `#[\Override]`, yet `implements-external-contract` never fires for any of them. An override of an Android, Flutter, UIKit or React base is therefore reported like ordinary dead code.
   - Verified: fixtures f1 and f2.
   - Code: `OVERRIDE_MARKERS` (mod.rs:409) already lists `@override`. The fix is in each plugin: put the modifier into `attributes`, as csharp.rs:332-342 does.
   - The same gap covers C++ `override` (cpp.md §12, `Ext::paintEvent`).

4. *Still true at 0.13.3 (fixture `m12/kh`); Kotlin now declares `value_refs` (0.11.1), but not `impls`, so `language-lacks-dispatch-model` still caps it.* **Kotlin is "analyzable" but can never produce `high`** (S–M). Java can.
   - Kotlin declares visibility and attributes (kotlin.rs:18-24) but not `value_refs` or `impls`. Inheritance fan-out runs for it anyway (dispatch.rs:114).
   - So the declared-signal bit, not the model, adds `language-lacks-dispatch-model` and `language-lacks-value-references`.
   - The report also states `max_confidence: high` for Kotlin (fixture f1, mod.rs:1149), which is wrong.
   - The same mismatch applies to Scala, Swift, Dart and Groovy (in `INHERITANCE_LANGS` but `impls` undeclared) and to Solidity.

5. **Go is permanently `degraded`** (S, a policy decision). `signals_complete` requires attributes (caps.rs:88-96), and Go has no attribute syntax. Go therefore cannot reach `high` although it has the visibility, value-ref and test-role signals Java has. Go also extracts reflection literals (go.rs:115) without declaring `dyn_uses`.

6. **Visibility is missing for languages with a visibility keyword or convention** (M each):
   - C++: `static`, anonymous namespace, access specifiers (cpp.md §12);
   - C: `static`, verified in f3;
   - PHP: `private`/`protected`/`public`, php.rs:239;
   - Swift;
   - Scala: `private`;
   - Dart: `_name`;
   - Ruby: `private` sections;
   - Elixir: `defp`, verified in f2;
   - Clojure: `defn-`;
   - F#: `private`;
   - Zig: `pub`;
   - Fortran: `private ::`;
   - Lean: broken capture, lean.rs:321-341;
   - Starlark: `_name`.

   Without it the language cannot be `analyzable`, and private dead code (the most certain kind) is capped at medium. C# and Java show that the model handles it.

7. *Fixed in 0.11.1 (fixture `m12/ov/python`).* **The Python `async def` external-contract miss** (S). The check `nd.kind != CallableKind::Function` (mod.rs:505) excludes async methods, which the plugin records with kind `function`. Verified: fixture f1 `a.P._ahook` is `high`, while its sync sibling `_shook` is `medium` with `implements-external-contract`. This is a false `high`, which is the band the tool promises is safe.

8. **Magic-method and metamethod roots** (S). `DUNDER_LANGS` includes PHP and Lua (roots.rs:160-161), but `is_dunder` needs a trailing `__` (roots.rs:967-969). PHP `__construct`/`__invoke`/`__call` and Lua `__index`/`__gc` are therefore never rooted. Python's equivalents are. Verified: f1 `C::__construct`.

9. **FFI export roots for C-family and native languages** (S–M). `classify_ffi` matches `extern:c`, `dllexport` and `jniexport` (ffi.rs:660-665), but no plugin emits those keys. Rust `#[no_mangle]` and C# `[UnmanagedCallersOnly]` are roots, while C/C++ `extern "C"` and `__declspec(dllexport)`, Zig `export fn`, Fortran `bind(c)`, Haskell `foreign export` and asm `.globl` are not. Verified: f3 `c_export` vs `N.E.Exp`.

10. **Visibility-scoped doubt for Java and C# only** (M). `can_reach` (mod.rs:655-680) is how a Java private member avoids doubt from an unrelated `x.save()`. Rust (`pub(crate)`/private), Kotlin (`private`/`internal`), Python (`_x`), Go (unexported) and C# `internal` keep the doubt from any same-named site in the tree.

11. **The reflection screen is missing for dynamic languages that have the concept** (S each, as a table row in dynuse.rs:26-56):
    - Kotlin (`::class.members`);
    - Groovy (`invokeMethod`, `this."$n"()`, groovy.md §12);
    - Perl (`can`, `->$name`);
    - Lua (`_G[...]`);
    - ObjC (`NSSelectorFromString`, `performSelector:`);
    - Swift (`#selector`);
    - Elixir/Erlang (`apply/3`);
    - Clojure (`resolve`);
    - R (`do.call`, `get`);
    - PowerShell (`& $name`);
    - Bash (`eval`, `"cmd_$1"`).

    JS/TS have only module-load screens, so `obj["m"]` and `Reflect.get(o, "m")` give no cap.

12. **Test tagging (`TestRole`) exists for Rust, Python and Go only** (M). Java, Kotlin, C#, JS/TS, Ruby and PHP have harness rules (matrix 11) but no in-file test role. So `only-used-by-tests` is withheld as "signal not extracted" (erlang.md, ocaml.md §12). A helper used only by tests is reported as production dead code or not reported at all.

13. **Clause-per-node languages report later clauses dead** (M). In Elixir, Erlang and Haskell multi-equation functions, only the first clause gets the edge. Evidence: elixir.md §12 (`transform` line 32), erlang.md §12 (`k:f@5`), haskell.md §12 (`areaOf@13`).

14. **Blind-spot disclosure covers 5 of 46 languages** (S). `BLIND_SPOTS` (caps.rs:44-68) has no entry for Bash (`trap`, `$(…)`), Groovy (paren-less), Erlang (`fun f/1`), Lean (typeclasses, notation), Swift (selectors), ObjC (selectors), C (function-pointer tables) or Go (interface satisfaction), although each has a documented, systematic false-positive class.

15. **Verilog `task` false root** (S). The plugin's kind tag `"task"` (verilog.rs:151-152) collides with Celery's `task` in `FRAMEWORK_ATTRS` (roots.rs:186-207), so every Verilog task is a root. The fix is either a language-gated attribute table or a non-colliding tag spelling.

16. *Still true at 0.13.3 for Fortran `PROGRAM` and Clojure `-main` (fixture `m12/misc`).* **Entry forms that are not `main`** (S each, as `LANG_ENTRY` rows or an attribute rule): Fortran `PROGRAM`, Clojure `-main`, F# `[<EntryPoint>]`, Swift `@main`, Verilog/VHDL top units, Kotlin `@JvmStatic main` in an object (unverified). Each one makes the whole language report `no-roots-found`, which withholds every reachability finding. Evidence: fortran.md, clojure.md, fsharp.md §12.

17. **Top-level invocation misses** (S–M):
    - Ruby paren-less top-level calls leave no audit record (ruby.md §12, `crates/cgg/src/lib.rs:1070-1086`).
    - R/Starlark top-level calls to another file's function have reason `no-candidate-in-file` / `candidates-in-other-files`, not `no-enclosing-callable`.
    - Lua/Perl module-qualified top-level calls do not match.
    - The rule matches only those two reasons (roots.rs:552-575).

18. **Doubt keyed by `(language, name)`** (M). Kivy KV sites cannot doubt Python methods (kivy.md §12, evidence.rs:102-126). A same-named-site check across a language pair that cgg links (KV→Python, XAML→C#) is the missing piece.

## Doc corrections

Corrections to the language docs (the docs are left unedited, per scope):

- **javascript.md §12**: "`reflection` caps come from `dyn_uses` (section 15)" is wrong for JS.
  - `REFLECTIVE` lists only `require` and `importScripts` for javascript (dynuse.rs:50).
  - Both are in `DYN_MODULE_LOADERS` (facts.rs:165), which yields the `module-load` screen. That screen does not cap (mod.rs:620-638, deadcode.rs:340-350).
  - JS has no reflection cap. Separately, javascript.rs:35-43 does not declare `dyn_uses` although javascript.rs:80 extracts it.
- **csharp.md §12**: "`ffi:export` from `FFI_EXPORT_ATTRS` contains no C# attribute. C# FFI exports are rooted through the `dotnet-ffi-export` framework rule instead."
  - This is half right. `FFI_EXPORT_ATTRS` has no C# entry, but `classify_ffi` (ffi.rs:663) matches `unmanagedcallersonly`, so `[UnmanagedCallersOnly]` is rooted as `ffi:export` ("exported over c-abi") *before* the framework entry.
  - Verified: fixture f3 lists both the `ffi:export` root `N.E.Exp` and the `dotnet-ffi-export` entry node.
- **c.md §12**: "No … lifecycle or conventional roots for C … `CONVENTIONAL_METHODS` are OO names." The table is applied to every language by simple name (roots.rs:874-886). Verified: fixture f3 C function `int describe(void)` was rooted `builtin:lifecycle` "`describe` is invoked by the runtime". It is an over-root, not an absent rule.
- **rust.md §12**: "The Rust visibility filter on doubt (unresolved sites must be able to reach the item)" is listed under confidence caps. In `crates/cgg-resolve/src/deadcode/` no Rust arm exists:
  - `can_reach` (mod.rs:655-680) handles only Java and C#;
  - the only Rust special case in evidence.rs (line 184) is the `super-base-out-of-graph` exclusion.
  - Treat the claim as unverified at best.
- **scala.md §12**: "Unverified whether `override def` of an out-of-tree base is capped." It is now verified **not** capped: fixture f2 `K::hook` in `class K extends Base` (`import ext.Base`) carries no `implements-external-contract`.
- **kotlin.md §12**: "`max_confidence: high`" is accurate as a report value, but the report value is wrong. The doc's own next section notes that every finding is medium. Fixture f1 `p.K.priv` carries both `language-signals-complete` and `language-lacks-*`. The inconsistency is in mod.rs:1149, which should be noted as a bug, not a capability.
- **go.md §12**: correct, with one addition. "dyn_uses not declared but extracted" also applies to JavaScript (javascript.rs:80 vs signals at :35-43), not only Go.
- **typescript.md §12**: "Unresolved-doubt visibility filtering is implemented only for Java and C#" is correct, but only for C# `private`. C# `internal` falls through to `true` (mod.rs:668-677).
- **CLAUDE.md** (reported here, not edited): it says "`dynuse` covers Python, JS, Java, C#, Ruby and PHP". `REFLECTIVE` (dynuse.rs:26-56) also has TypeScript and Go rows, and the JS/TS rows are module-load only.
- **caps.rs:3-4 comment** (code comment, not a language doc): "dispatch modelling only produces edges for Rust" is stale. `inheritance_fanout` produces edges for the 12 `INHERITANCE_LANGS` under `--dead-code` (dispatch.rs:110-123), and Groovy fixture f1 in groovy.md shows `Base.area → Circle.area`.

## Fixtures run for this matrix

All of them are under `<scratch>/parity/matrix-12/`, run with `cgg . --dead-code --dead-code-confidence low --dead-code-format json --no-graph -o out.json` and read from `out.json.deadcode.json`:

- **f1** (Kotlin, TS, Python, PHP, Go, Java, Swift; external bases and `override` forms):
  - only the Java/Python private findings are `high`;
  - Python `_ahook` (async) is `high` without a contract;
  - Kotlin, TS, Swift and PHP overrides carry no contract;
  - the Kotlin report says `max_confidence high`;
  - PHP `__construct` is a finding.
- **f2** (Scala, Dart, Groovy, Ruby, Elixir):
  - Groovy `@Override` is capped (`implements-external-contract:Base`);
  - Scala, Dart and Ruby are not;
  - Elixir `def` and `defp` get identical evidence.
- **f3** (C#, C++, C):
  - `[UnmanagedCallersOnly]` gives `ffi:export`;
  - `extern "C"` and C++/C `static` give plain medium findings;
  - a C `describe()` is rooted by `CONVENTIONAL_METHODS`.
