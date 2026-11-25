# 17. External dependencies (`--externals`)

> Re-checked against 0.13.3 (2026-11-25): the rung table against
> `ExternalEvidence::confidence`, the Process / Native / System columns
> against `PROCESS_SPAWN_APIS`, `LIBRARY_LOAD_APIS` and
> `OS_BINDING_MODULES`, and the row set against
> `crates/cgg/tests/externals.rs` (45 rows plus `graphql` in
> `EXTERNALS_NA`). The Imports column was not re-verified beyond what
> those tests assert. R, PowerShell and asm were re-run on fixtures.

This category covers what `--externals` can attribute in each language:
which import forms classify a library as internal, stdlib, system or
external; how a call leaving the tree is tied to one; and which process,
native and system forms are recognised. Resolution itself is out of scope
(section 04); this pass only reads what the resolvers left in the
`stdlib_calls` / `external_calls` buckets.

Sources: `crates/cgg-resolve/src/externals.rs` (attribution),
`crates/cgg-core/src/externals.rs` (kinds, evidence rungs and their
confidence), `crates/cgg/src/externals_report.rs` (the `cgg.externals.v1`
report), `crates/cgg-lang/src/externals.rs` (side-channel extraction),
`crates/cgg-core/src/stdlib.rs` (`STDLIB_ROOTS`, `PROCESS_SPAWN_APIS`,
`LIBRARY_LOAD_APIS`, `OS_BINDING_MODULES`, `SYSTEM_HEADERS`,
`SYSTEM_APIS`, `SYSCALL_MNEMONICS`, `SHELL_BUILTINS`) and the per-language
fixtures in `crates/cgg/tests/externals.rs`, which every row below has.

## How a call is attributed

Strongest evidence first; the first rung that applies wins.

| Rung | Evidence | Confidence |
| --- | --- | --- |
| 1 | `resolver-refusal`: cross-file filed the call `external-module` | high |
| 2 | `qualified-import`: the receiver head / qualifier is bound by an import, or the language names the module outright (Erlang `m:f`, Elixir `Mod.f`, OCaml `M.f`, Perl `Pkg::f`, R `pkg::f`, Rust `crate::path`) | high |
| 3 | `selective-import`: a bare name the file imports by name | high |
| 4 | `foreign-declaration` / `literal-argument` / `system-interface`: FFI declarations, literal programs or libraries, syscall instructions and OS API tables | high |
| 5 | `typed-receiver`: a receiver type bound by an import (low when propagation guessed it) | medium |
| 6 | `chain-head`: a chain headed by an imported module or type | medium |
| 7 | `name-prefix`, `undefined-instance`, `stdlib-screen` | medium |
| 8 | `sole-wildcard`: the only external wildcard / open import | low |
| 9 | none → `unattributed` | low |

## Kinds and the report

`ExternalKind` (`crates/cgg-core/src/externals.rs`) has six kinds:
`third-party`, `stdlib`, `system`, `native`, `process` and
`unattributed`. Bare `--externals` shows
`third-party,system,native,process`; `stdlib` and `unattributed` are
counted as withheld, never dropped (`cgg --help`). Evidence confidence is
`ExternalEvidence::confidence`: rungs 1–4 high, 5–7 medium, 8–9 low, and a
`typed-receiver` attribution drops to low when the receiver type was a
propagation guess (`weak_receiver_type`). The JSON report's top-level keys
are `schema`, `cgg_version`, `disclaimer`, `kinds`, `totals`, `coverage`,
`libraries` and `unattributed` (0.13.3 fixture run).

## The matrix

| # | Language | Imports read | Process | Native / FFI | System | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | rust | `use`, path-written crates | `Command::new` | `#[link]` extern blocks, `libloading` | `libc`, `nix`, `windows_sys`, `windows`, `winapi`, `rustix`, `syscalls` | An unimported capitalized head is a type of the module, never a crate |
| 2 | python | `import`, `from … import` (heads.rs judges) | `subprocess.*`, `os.system`, `os.exec*` | `ctypes`, `cffi` | `posix`, `fcntl`, `mmap`, … | |
| 3 | kivy | `#:import`, `#:include` (side channel) | as python | as python | as python | |
| 4 | javascript | `import`, `require` (heads.rs judges) | `child_process.*`, `execa`, `Bun.spawn`, `Deno.Command` | `ffi-napi`, `koffi`, `process.dlopen` | `node:os` | Library = npm package root |
| 5 | typescript | as javascript | as javascript | as javascript | as javascript | |
| 6 | go | `import` (heads.rs judges; stdlib = no dot in first element) | `exec.Command` | cgo `C.f`, `syscall.LoadDLL`, `plugin.Open` | `syscall`, `golang.org/x/sys` | Library = module root by host heuristic |
| 7 | java | `import` | `Runtime.exec`, `ProcessBuilder` | `native` methods, `System.loadLibrary`, JNA | — | Library = namespace root |
| 8 | kotlin | `import` | as java | `external`, `System.loadLibrary` | — | |
| 9 | c | `#include` (ISO vs POSIX/Win32 vs third-party headers) | `system`, `popen`, `exec*`, `posix_spawn`, `CreateProcess` | `dlopen`, `LoadLibrary` | `SYSTEM_APIS` (POSIX, Win32) | A third-party call is tied to an included header by name prefix |
| 10 | cpp | as c, plus `ns::f` paths | as c, `QProcess` | as c, `QLibrary` | as c | |
| 11 | csharp | `using` (wildcard) | `Process.Start` | `DllImport`, `LibraryImport`, `NativeLibrary.Load` | `Microsoft.Win32` | |
| 12 | bash | `source` | every non-builtin command | — | — | Builtins (`SHELL_BUILTINS`) are stdlib |
| 13 | ruby | `require` | `system`, `spawn`, `Open3`, backticks | `ffi_lib`, `Fiddle` | `Etc`, `Fcntl` | |
| 14 | swift | `import` | `Process`, `posix_spawn` | `dlopen` | `Darwin`, `Glibc` | Unimported type heads go to the only external import |
| 15 | lua | `require` | `os.execute`, `io.popen` | LuaJIT `ffi.load` | `os` | |
| 16 | php | `use`, `require` | `exec`, `shell_exec`, `proc_open`, backticks | `FFI::cdef`, `FFI::load` | — | Library = `Vendor\\Package` |
| 17 | dart | `import` (`package:`, `dart:`) | `Process.run`/`start` | `DynamicLibrary.open` | `dart:io` | |
| 18 | scala | `import` | as java | `System.loadLibrary` | — | |
| 19 | hcl | `module` `source`, `provider`, `required_providers` (side channel) | — | — | — | Providers and remote modules are third-party |
| 20 | zig | `@import` | `std.process.Child` | `extern "lib" fn`, `@cInclude` | `std.os`, `std.posix` | |
| 21 | objc | `#import`, `#include` | `NSTask`, `system` | `dlopen` | as c | |
| 22 | r | `library`, `require`, `pkg::f` | `system`, `system2`, `shell`, `processx::run` | `dyn.load`, `library.dynam`, `useDynLib` | — | `.Call("sym", …)` / `.C(…)` are not attributed (0.13.3 fixture: `unattributed`) |
| 23 | groovy | `import` | as java, `"cmd".execute()` | as java | — | |
| 24 | julia | `using`, `import` | `run` | `ccall`, `dlopen` | `Base.Libc` | |
| 25 | perl | `use`, `require` | `system`, `exec`, `qx`, backticks | `XSLoader`, `DynaLoader` | `POSIX` | |
| 26 | elixir | `alias`, `import`, `use`, `Mod.f` | `System.cmd`, `Port.open` | `:erlang.load_nif` | — | |
| 27 | erlang | `m:f`, `-import`, `-include_lib` | `os:cmd`, `open_port` | `erlang:load_nif` | `os` | OTP applications are stdlib |
| 28 | fortran | `use` | `execute_command_line` | `bind(c)` | — | Intrinsic modules are stdlib |
| 29 | clojure | `:require`, `:import` | `clojure.java.shell/sh` | `System/loadLibrary` | — | |
| 30 | haskell | `import` (+ qualified aliases) | `System.Process` | `foreign import` | `System.Posix`, `System.Win32` | `base` modules listed one by one |
| 31 | ocaml | `open`, `M.f` | `Unix.system`, `Sys.command` | `external` | `Unix` | |
| 32 | powershell | `Import-Module`, dot-source | native commands | `Add-Type`, `[…NativeLibrary]::Load` | — | Verb-Noun cmdlets are stdlib. `Start-Process "x"` is listed in `PROCESS_SPAWN_APIS` but a 0.13.3 fixture attributes it `stdlib` (`Microsoft.PowerShell`, `stdlib-screen`), not `process` |
| 33 | solidity | `import` (`@scope/pkg`) | — | — | — | Calls through an imported interface |
| 34 | fsharp | `open` (wildcard) | `Process.Start` | `DllImport` | — | |
| 35 | starlark | `load("@repo//…")` | `genrule`, `ctx.actions.run` | — | — | `//…` labels are internal |
| 36 | cmake | `find_package`, `include` | `execute_process`, `add_custom_command` | — | — | |
| 37 | nix | `import <…>`, fetchers (side channel) | `runCommand` | — | — | `fetchFromGitHub` → `github:owner/repo` |
| 38 | verilog | `` `include ``, `import pkg::*`, DPI-C (side channel) | — | DPI-C | — | Instances of undefined modules are third-party IP |
| 39 | vhdl | `library`, `use` | — | — | — | `ieee`/`std` stdlib, `work` internal |
| 40 | asm | `.extern`, `.include` (side channel) | — | — | `syscall`, `int 0x80`, `svc`, `ecall` | `call` into libc via the C tables |
| 41 | smithy | `use a#X` (side channel) | — | — | — | `smithy.api` is stdlib |
| 42 | proto | `import "x.proto"` (side channel) | — | — | — | `google/protobuf/*` is stdlib |
| 43 | graphql | — | — | — | — | `EXTERNALS_NA`: SDL has no import mechanism |
| 44 | openapi | external `$ref` (side channel) | — | — | — | |
| 45 | asyncapi | external `$ref` (side channel) | — | — | — | |
| 46 | lean | `import` | `IO.Process` | `@[extern]`, `Lean.loadDynlib` | — | `Init`/`Lean`/`Std` are stdlib |

## Known gaps

- A call through a value whose type cgg never sees is `unattributed`.
  `--externals=all` lists those calls by name. The share is large for
  method-heavy code in languages without receiver typing.
- A process or library literal built at run time is reported as
  `<dynamic>`: a variable, a concatenation or a macro (`env!(…)`).
- Haskell qualified calls (`T.pack`) are not call sites in the plugin,
  so the externals walk records them in the side channel
  (`qualified-apply`). They count here but are still not edges.
- No dependency manifest is read, by design.
