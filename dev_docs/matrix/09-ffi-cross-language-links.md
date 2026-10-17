# 09. FFI & cross-language links

This category covers every edge cgg draws across a language boundary, and every marker that makes a symbol live because a caller in another language or process uses it. It has five mechanisms:

1. `ffi::link_ffi` (`crates/cgg-resolve/src/ffi.rs`):
   - **Pass A** is the asm ↔ C-family bridge (ffi.rs:36-175).
   - `link_kivy_python` covers `.kv` markup → Python (ffi.rs:178, 268-560).
   - **Pass B** is the attribute-driven family linker (ffi.rs:188-262). It runs `classify_ffi` (ffi.rs:643-677) and applies `allowed_source_languages` (ffi.rs:694-701).
2. `descriptor::link_descriptors` (`crates/cgg-resolve/src/descriptor.rs`) links an interface declaration (proto rpc, Smithy, GraphQL) to the host-language method that implements it.
3. Markup code-behind (`crates/cgg-lang/src/plugins/markup.rs` and `markup_kinds_for` in `rules.rs:11812-11819`) roots a C# handler that a sibling `.xaml`, `.razor` or `.tscn` file binds.
4. The FFI-export liveness markers: the framework rules of `TrustKind::Ffi`, and `RootKind::FfiExport` in `deadcode/roots.rs:763-779`.
5. Sibling-language resolution that is not FFI but does cross a language boundary: the C/C++/Obj-C include family, and Elixir → Erlang `:mod.f`.

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | full: the idiomatic form links, or roots, as expected |
| 🟡 | partial: classified only, works only under a naming accident, one direction only, or emits phantom edges |
| ⬜ | none: the language has the concept and cgg does nothing with it |
| — | n/a: the concept does not exist for this language, or the language cannot be on that side of the boundary |

### Columns

Each cell describes **this language's side** of the mechanism.

| Column | Meaning |
| --- | --- |
| **C-ABI** | `c-abi` family. The export side is Rust `#[no_mangle]`/`extern "C"`/`export_name` or C# `[UnmanagedCallersOnly]`/`[DllExport]`. The caller side must be in the C family (ffi.rs:698). For C-like languages, the cell also covers the language's own way of exporting a C symbol (`__declspec(dllexport)`, `//export`, `export fn`, `bind(c)`, `@_cdecl`). |
| **PyO3** | Rust `#[pyfunction]`/`#[pymethods]`/`#[pyclass]` linked to a Python callable (ffi.rs:657, 699). |
| **napi** | Rust `#[napi]`/`#[module_exports]` linked to a JS/TS callable (ffi.rs:697). |
| **wasm-bg** | Rust `#[wasm_bindgen]`. Unrestricted: any language's same-named callable can be the caller (`allowed_source_languages` returns `None`, ffi.rs:700). |
| **UniFFI** | Rust `#[uniffi::export]`. Unrestricted, like wasm-bg. |
| **JNI** | Java/Kotlin/Scala `native`/`external` linked to a C/C++/Rust `Java_pkg_Cls_m`. `classify_ffi` has `native` → `jni` Import (ffi.rs:668) and `jniexport` → `jni` Export (ffi.rs:664). |
| **P/Invoke** | `[DllImport]` → `pinvoke` Import (ffi.rs:667), or the C side it targets. |
| **asm↔C** | Pass A, which links asm labels and C-family functions by name in both directions. |
| **Sibling** | Same-family cross-language resolution outside `link_ffi`: C/C++/Obj-C through includes, Elixir↔Erlang, JVM, .NET, Swift↔Obj-C. |
| **Native FFI** | The language's *own* native-interop construct is recognised and linked to the other side. Examples: cgo `C.f`, ctypes/cffi, `dart:ffi`, `ccall`, Haskell `foreign import`, OCaml `external`, NIFs, DPI-C, `.Call`/Rcpp, LuaJIT `ffi.C`, `FFI::cdef`, XS. For build languages it means a build target linked to the sources it names. |
| **FFI root** | An exported symbol is a dead-code root, or mints a framework entry (`TrustKind::Ffi` rules `ffi-export` rules.rs:1445, `zig-export` 6613, `dotnet-ffi-export` 9379, `blazor-jsinterop` 11153). |
| **Desc host** | A Protobuf `rpc` links to an idiomatic implementation in this language (descriptor.rs:59-157: the method name must equal the rpc name case-insensitively, and the owner must contain the service name). |
| **Desc source** | This language is a descriptor language whose operations produce `Via::Descriptor` edges (`DESCRIPTOR_LANGUAGES`, descriptor.rs:47). |
| **Markup** | Markup or template event bindings reach host code (C# XAML/Razor/tscn; Kivy kv; Android XML, storyboards, Angular templates and the like). |

### Fixtures run for this matrix

They live under `<scratch>/parity/matrix-09/`. Each was run with `target/release/cgg . -t json -o out.json`, and the non-direct edges were printed with `show.py`.

- **d1**: one `service Greeter { rpc SayHello; rpc Ping }` plus one idiomatic `SayHello`/`sayHello` implementation in each of Dart, Swift, Ruby (both `say_hello` and `SayHello`), Rust (trait impl `say_hello` and inherent `sayhello`), Elixir (`Hw.Greeter.Server`), Kotlin, Perl, Lua, Scala, Groovy, OCaml, F#, Obj-C, Julia and Clojure.
- **d2**: `rpc Ping` against Elixir `Hw.GreeterServer.ping`, Lua `GreeterService:Ping`, Rust `impl Greeter for MyGreeter { fn ping }`, OCaml `module GreeterImpl = struct let ping`, and Haskell `module GreeterImpl where ping`.
- **d3**: Erlang `greeter_impl:ping`; Java `native compute` with a C `JNIEXPORT … Java_p_N_compute`; a C `__declspec(dllexport) exported_c` *definition* with a Rust `#[no_mangle] extern "C" exported_c`; Rust `#[pyo3::pyfunction]` and `#[pyfunction]` with Python twins.
- **d4**: Rust `#[uniffi::export] greet`/`add_nums` and `#[wasm_bindgen] wfun`; Kotlin `greet`/`addNums`/`wfun`; Swift `greet`; a C prototype `wfun`.

## The matrix

| # | Language | C-ABI | PyO3 | napi | wasm-bg | UniFFI | JNI | P/Invoke | asm↔C | Sibling | Native FFI | FFI root | Desc host | Desc source | Markup | Notes (most important gap) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | rust | ✅ export side: bare `#[no_mangle]`, `extern "C"` (`extern:C`, rust.rs:814), `export_name` linked from a C caller (d3) | 🟡 bare `#[pyfunction]` only. `#[pyo3::pyfunction]` gets no edge (d3), and `#[pymethods]` members get no edge | 🟡 exact name only, no snake→camel (JS doc f7) | 🟡 bare attr only; unrestricted, so Kotlin and C twins produce phantom edges (d4) | 🟡 unrestricted name twin, no snake→camel (`add_nums` ≠ `addNums`, d4) | ⬜ `extern "system" fn Java_…` is seen only as c-abi via `no_mangle`; no Java link (rust doc §09) | — | ⬜ Rust is not in `language_family("c")`, so `extern "C"` callers never reach asm (lang_family.rs:26-37) | — | 🟡 `#[link]` is classified c-abi Import (ffi.rs:671); an `extern "C" { fn f(); }` block never links to the C body | ✅ `ffi-export` rule (rules.rs:1445), the `pymethods`/`uniffi::export` container (rules.rs:11776), and classify_ffi exports → `ffi:export` root (roots.rs:763) | 🟡 a trait impl owner works (d2 `<MyGreeter as Greeter>::ping`), but idiomatic tonic `say_hello` never matches `SayHello` (d1) | — | — | Path-qualified FFI attributes (`#[pyo3::pyfunction]`, `#[wasm_bindgen::prelude::wasm_bindgen]`) are never Pass-B linked, because classify_ffi compares the whole key (ffi.rs:647-653) |
| 2 | python | ⬜ ctypes/cffi → C: c-abi accepts only C-family callers (ffi.rs:698) | 🟡 Python side: a callable-to-callable twin needs a Python `def`/`.pyi`; a call site with no stub gets no edge (python doc f9) | — | 🟡 unrestricted; a Python twin links (rust doc ws3), often a phantom | 🟡 unrestricted name twin (by code, ffi.rs:700) | — | — | — | — | ⬜ ctypes, cffi, Cython (`.pyx` not detected) | — | ✅ `GreeterServicer.SayHello` (python doc f10, proto doc fixture b) | — | ✅ via Kivy kv → Python (`link_kivy_python`) | PyO3 calls from Python sites are linked only when a same-named Python stub exists; there is no call-site → export edge |
| 3 | kivy | — | — | — | — | — | — | — | — | — | — | — | — | — | ✅ `kv-python` linker with owner scoring, fanout cap and audited refusals (ffi.rs:268-447; tests/kivy.rs) | None in this category. The kv linker is the most complete cross-language pass cgg has |
| 4 | javascript | — | — | 🟡 exact-name twin only; `#[napi] sum_values` ≠ `sumValues` (JS doc f7) | 🟡 exact-name twin (test `wasm_bindgen_links_rust_to_javascript`, ffi.rs:910) | — | — | — | — | — | ⬜ `require('./x.node')` → `not-in-imported-module` (JS doc f7) | — | ✅ `GreeterService.sayHello` (JS doc f8) | — | ⬜ JSX/Vue/Svelte/HTML templates not read | No snake_case → camelCase mapping for napi, which is napi's default renaming, so most real napi exports never link |
| 5 | typescript | — | — | 🟡 same as JS; `sum_nums` ≠ `sumNums` (TS doc f12) | 🟡 exact-name twin (TS doc f12) | — | — | — | — | — | ⬜ native addons not followed | — | ✅ (TS doc f13) | — | ⬜ Angular/JSX templates not read (TS doc §09) | Same napi camelCase gap as JS. Angular template bindings are invisible |
| 6 | go | ⬜ `//export GoCallback` is not an export: the C call binds the C prototype (go doc f7) | — | — | 🟡 unrestricted twin possible (by code) | 🟡 unrestricted twin possible (by code) | — | — | ⬜ Go asm (`.s` with `TEXT ·f`) is not linked to Go (Go not in the C family) | — | ⬜ cgo `C.add()` → external bucket (go doc f7) | ⬜ `//export` is not a root; there is no Go `TrustKind::Ffi` rule (rules.rs grep) | 🟡 `GreeterServer.SayHello` links, but the grpc-go idiom `type server struct` does not, though it is descriptor.rs:10's own example (go doc f7) | — | ⬜ `html/template` not parsed | cgo is wholly unlinked in both directions. The descriptor rule fails on the canonical `server` struct |
| 7 | java | — | — | — | 🟡 unrestricted twin possible | 🟡 unrestricted twin possible | 🟡 `native` → `jni` Import is classified (java.rs:270-281; ffi.rs:668), but no edge to `Java_p_N_compute` (d3) and no root (java doc f3) | — | — | ⬜ JVM family disabled (lang_family.rs:32) | — | ⬜ a `native` method is reported dead (java doc f3) | ✅ `GreeterImpl.sayHello` (test `java_lowercases_the_first_letter_and_still_links`, descriptor.rs:205) | — | ⬜ Android XML `android:onClick` not read | JNI is classified but never linked: imports never get Pass-B edges (ffi.rs:678-686), and no plugin records `JNIEXPORT` (d3) |
| 8 | kotlin | — | — | — | 🟡 unrestricted: Kotlin `wfun` → Rust `#[wasm_bindgen] wfun` phantom (d4) | 🟡 `greet` links; `add_nums` ≠ `addNums` (d4) | ⬜ `external fun` is not recorded as an attribute (kotlin doc f5) | — | — | ⬜ no Java/Kotlin interop (lang_family.rs:32) | — | ⬜ `external` not rooted | ✅ `GreeterImpl.sayHello` (d1) | — | ⬜ Android XML / data binding not read | `external fun` gets nothing that Java `native` gets (not even the classification) |
| 9 | c | 🟡 caller side ✅ (prototype `rs_add@ffi.h` → Rust, c doc f8); export side ⬜ (`__declspec(dllexport)` not recorded, c.rs:216/247 `attributes: Vec::new()`); a C *definition* of the same name is drawn as a caller of the Rust export, a phantom edge (d3) | — | — | 🟡 a C prototype `wfun` is drawn as a caller of Rust `#[wasm_bindgen] wfun` (d4) | 🟡 unrestricted | ⬜ `JNIEXPORT … Java_p_N_compute` gets no attribute and no link (d3) | ⬜ C# `[DllImport] add` never reaches C `add` (csharp doc f6) | ✅ both directions (tests `a_value_reference_to_an_asm_label_is_not_an_ffi_call`, `an_asm_call_strips_at_most_one_underscore_and_only_when_needed`) | ✅ C ↔ C++ ↔ Obj-C by include closure (cpp doc §09) | — | ⬜ `dllexport`/`visibility("default")` not roots | — (free functions have no owner; descriptor.rs:122-127 requires one) | — | — | The C plugin records no attributes, so C can never be an FFI export or a JNI target |
| 10 | cpp | 🟡 same as C: the caller side is in the c family; `extern "C"` and `dllexport` exports are not recorded and are reported dead (cpp doc f9) | — | — | 🟡 unrestricted | 🟡 unrestricted | ⬜ no link to `Java_com_x_Native_compute` (cpp doc f10) | ⬜ | ✅ both directions (cpp doc f10) | ✅ include family | ⬜ pybind11/nanobind/Boost.Python/node-addon-api/Emscripten `EMSCRIPTEN_KEEPALIVE` | ⬜ no C++ FFI rule | ✅ `GreeterServiceImpl::SayHello` (cpp doc f14) | — | — | `extern "C"` is not captured as an attribute, unlike Rust's (rust.rs:814). pybind11 `m.def("name", &fn)` is not linked to Python |
| 11 | csharp | ✅ export side: `[UnmanagedCallersOnly]`/`[DllExport]` linked from a C prototype (csharp doc f6) | — | — | — | — | — | 🟡 `[DllImport]` is classified Import with no edge to C (csharp doc f6); `[LibraryImport]` is not classified (ffi.rs:653-672) | — | ⬜ C#↔F# family disabled (lang_family.rs:34) | ⬜ COM, `delegate* unmanaged`, `Marshal.GetDelegateForFunctionPointer` | ✅ `dotnet-ffi-export` (rules.rs:9379), `blazor-jsinterop` (rules.rs:11153) | ✅ `GreeterImpl.SayHello` (csharp doc f6) | — | 🟡 XAML/AXAML, Razor and Godot `.tscn` bindings → `ui` framework entries only, never an edge (markup.rs:1-12; test `a_xaml_binding_enters_its_handler_keyed_by_the_event`) | P/Invoke (`DllImport`/`LibraryImport`) never links to the in-tree C body, so the C function looks dead |
| 12 | bash | — | — | — | — | — | — | — | — | — | ⬜ `python x.py`, `node x.js`, `./other.sh` → external, script arg dropped (bash doc §09) | — | — | — | — | Script → program/script invocation is never linked |
| 13 | ruby | — | — | — | 🟡 unrestricted | 🟡 unrestricted (UniFFI targets Ruby) | — | — | — | — | ⬜ `rb_define_method`, ffi gem `attach_function`, Fiddle, magnus/rutie (ruby doc §09) | ⬜ | 🟡 only a CamelCase `def SayHello` links; idiomatic `say_hello` does not (d1) | — | ⬜ ERB not read | C extensions (`rb_define_method(klass,"name",c_fn,n)`) are never linked to Ruby call sites |
| 14 | swift | ⬜ `@_cdecl("name")` not recognised (swift doc f11) | — | — | 🟡 unrestricted | 🟡 `greet` → Rust `#[uniffi::export] greet` (d4); no camelCase mapping | — | — | — | ⬜ no Swift↔Obj-C bridge (lang_family.rs:28) | ⬜ bridging-header C calls → `no-candidate-in-file` (swift doc f11) | ⬜ | ✅ `GreeterProvider.sayHello` (d1) | — | ⬜ storyboard/XIB `IBAction` not read | No Swift ↔ C/Obj-C interop of any kind |
| 15 | lua | — | — | — | 🟡 unrestricted | 🟡 unrestricted | — | — | — | — | ⬜ LuaJIT `ffi.C`, `luaopen_*`, `luaL_Reg` (lua doc §09) | ⬜ | 🟡 `GreeterService.SayHello` links; colon methods `GreeterService:Ping` never do, because `split_last_segment` knows only `.` and `::` (names.rs:81-91; d1, d2) | — | — | `luaopen_x` in C is not tied to `require("x")`. Colon-defined methods never get descriptor links |
| 16 | php | — | — | — | 🟡 unrestricted | — | — | — | — | — | ⬜ `FFI::cdef`, `PHP_FUNCTION` (php doc §09) | ⬜ | ✅ `GreeterService::SayHello` (php doc p11) | — | ⬜ Blade/Twig not read | PHP `FFI::cdef()` and Zend extensions are invisible |
| 17 | dart | — | — | — | 🟡 unrestricted | — | — | — | — | — | ⬜ `dart:ffi` `lookupFunction`/`@Native`, `MethodChannel.invokeMethod` (dart doc §09) | ⬜ | ✅ `GreeterService::sayHello` (d1; the dart doc is wrong) | — | — | Flutter platform channels and `dart:ffi` never cross into Kotlin/Swift/C |
| 18 | scala | — | — | — | 🟡 unrestricted | — | 🟡 `@native` classifies as `jni` Import by code (classify_ffi trims `@`, ffi.rs:647-650); no edge, like Java | — | — | ⬜ JVM family disabled | — | ⬜ | ✅ `GreeterImpl::sayHello` (d1, scala doc f4) | — | — | No JVM interop with Java in the same tree |
| 19 | hcl | — | — | — | — | — | — | — | — | — | ⬜ resource → Go provider (hcl doc §09) | — | — | — | — | Terraform resources are never linked to provider Go code |
| 20 | zig | ⬜ `export fn` called from C binds the C prototype; `extern fn` binds the Zig prototype (zig doc p5) | — | — | 🟡 unrestricted | — | — | — | ⬜ Zig not in the C family | — | ⬜ `@export`, `callconv(.c)` (zig doc p1) | 🟡 the `zig-export` rule exists but has `attributes: NONE` and a stated gap (rules.rs:6613-6628), so nothing is rooted | — | — | — | Zig plugin records no attributes or visibility, so `export fn` is neither linked nor rooted |
| 21 | objc | ⬜ objc records no C calls, so a Rust c-abi export called from `.m` gets no edge (objc doc f4) | — | — | 🟡 | — | — | — | 🟡 asm → objc ✅ (objc doc f4); objc → asm ⬜ (C calls are not recorded) | 🟡 C → objc ✅; objc → C ⬜ (objc doc f4); Swift bridge ⬜ | — | ⬜ | ✅ `GreeterService::sayHello` (d1) | — | ⬜ XIB/storyboard not read (rules.rs:5398 gap text) | Plain C calls inside `.m` are not extracted, so every outgoing cross-language edge is missing |
| 22 | r | — | — | — | — | — | — | — | — | — | ⬜ `.Call`/`.C`/`.External`, Rcpp `[[Rcpp::export]]`, reticulate (r doc §09) | ⬜ | — (no owners on functions; unverified for R5/R6 classes) | — | — | Rcpp exports are never linked to `RcppExports.R` callers |
| 23 | groovy | — | — | — | 🟡 unrestricted | — | — | — | — | ⬜ JVM family disabled (lang_family.rs:32) | — | ⬜ | ✅ `GreeterImpl.sayHello` (d1; the groovy doc is wrong) | — | — | No Groovy → Java interop |
| 24 | julia | — | — | — | 🟡 unrestricted | — | — | — | — | — | ⬜ `ccall`/`@ccall`, PyCall, CxxWrap (julia doc §09) | ⬜ | 🟡 links through the *module* as owner (`module Greeter; sayhello(x)` → linked, d1). The owner is a module, not a type, so a match is a naming accident | — | — | `ccall((:f, lib), …)` is never linked to an in-tree C `f` |
| 25 | perl | — | — | — | 🟡 unrestricted | — | — | — | — | — | ⬜ XS (`.xs` not detected), Inline::C, FFI::Platypus | ⬜ | ✅ `package GreeterService; sub SayHello` (d1; the perl doc is wrong) | — | — | XS glue is not detected at all |
| 26 | elixir | — | — | — | 🟡 unrestricted | — | — | — | — | ✅ Elixir → Erlang `:mod.f` at high (libscope.rs:1095-1108) | ⬜ Rustler `#[rustler::nif]`, `:erlang.load_nif`, ports | ⬜ | 🟡 needs the *last* module segment to name the service (`Hw.GreeterServer` ✅, d2; `Hw.Greeter.Server` ⬜, d1) and fails on idiomatic snake_case `say_hello` (d1) | — | — | Rustler NIFs: no link between `#[rustler::nif] fn add` and the Elixir stub `def add` |
| 27 | erlang | — | — | — | 🟡 unrestricted | — | — | — | — | 🟡 Erlang → Elixir `'Elixir.Foo':bar()` → external (erlang doc f9) | ⬜ NIF `-nifs`, `erlang:load_nif`, `enif_*` C, ports | ⬜ | ⬜ `greeter_impl:ping` never links: no owner can be derived from `mod:f` (names.rs:81-91; d3) | — | — | Erlang NIF stubs are never linked to their C `enif` bodies |
| 28 | fortran | ⬜ `bind(c, name=…)` not read (fortran.rs:171 `attributes: Vec::new()`); C's call binds its own prototype (fortran doc fixture f) | — | — | 🟡 unrestricted | — | — | — | ⬜ not in the C family | — | ⬜ `bind(c)` interface → C body; f2py | ⬜ `bind(c)` not rooted | — | — | — | `bind(c)` is the whole Fortran/C interop story and it is not read |
| 29 | clojure | — | — | — | 🟡 unrestricted | — | — | — | — | ⬜ Java interop and ClojureScript `js/` → external (clojure doc p1) | — | ⬜ `^:export` lands in the name, not an attribute (clojure doc §09) | 🟡 links only through the namespace head acting as owner (`greeter.service/SayHello`, owner `greeter`, d1); idiomatic kebab-case `say-hello` cannot match | — | — | Java interop (`(.method obj)`, `gen-class`) is never linked to in-tree Java |
| 30 | haskell | — | — | — | 🟡 unrestricted | — | — | — | — | — | ⬜ `foreign import ccall` → external; `foreign export` not recorded (haskell doc f2) | ⬜ `foreign export` not rooted | 🟡 the module acts as owner (`module GreeterImpl where ping` → linked, d2); a naming accident, not a type match | — | — | `foreign import`/`foreign export` are ignored |
| 31 | ocaml | — | — | — | 🟡 unrestricted | — | — | — | — | — | ⬜ `external … = "caml_add"` not extracted (ocaml doc f1) | ⬜ | ⬜ qualified names carry no module prefix (`module GreeterImpl = struct let ping` → qn `ping`, d2), so there is never an owner | — | — | `external` stubs are not extracted, so `caml_*` C functions look dead |
| 32 | powershell | — | — | — | — | — | — | ⬜ `[DllImport]` inside `Add-Type` (unverified by fixture) | — | ⬜ C# `[Cmdlet]` class ↔ `Get-Foo` call | ⬜ `Add-Type -TypeDefinition`, script invocation | — | unverified (PowerShell class methods were not tested) | — | — | C#-implemented cmdlets are never linked to their verb-noun call sites |
| 33 | solidity | — | — | — | — | — | — | — | — | — | ⬜ ABI → ethers/web3/typechain callers (solidity doc §09) | — | — | — | — | Contract ABI is never linked to JS/TS/Python callers |
| 34 | fsharp | — | — | — | 🟡 unrestricted | — | — | ⬜ `[<DllImport>] extern` produces no callable; the call is `no-candidate-in-file` (fsharp doc f8) | — | ⬜ C# ↔ F# disabled (lang_family.rs:34) | — | ⬜ | ✅ `GreeterImpl.SayHello` (d1; the fsharp doc is wrong) | — | ⬜ Avalonia/WPF F# code-behind not read (`markup_kinds_for` is C#-only) | `extern` P/Invoke declarations are not even callables, which C# at least has |
| 35 | starlark | — | — | — | — | — | — | — | — | — | ⬜ `cc_library(srcs=…)` → sources (starlark doc §09) | — | — | — | — | Build targets are never linked to their `srcs` |
| 36 | cmake | — | — | — | — | — | — | — | — | — | ⬜ `add_library(foo foo.c)`, `-P script.cmake` (cmake doc §09) | — | — | — | — | No CMake → source or script link |
| 37 | nix | — | — | — | — | — | — | — | — | — | ⬜ `builder = ./build.sh`, `runCommand` shell (nix doc §09) | — | — | — | — | Builder scripts are not linked |
| 38 | verilog | — | — | — | — | — | — | — | — | ⬜ mixed Verilog/VHDL instantiation | ⬜ DPI-C import/export (verilog doc fixture `c/p6_dpi.sv`) | ⬜ `export "DPI-C"` not rooted | — | — | — | DPI-C is never linked to C |
| 39 | vhdl | — | — | — | — | — | — | — | — | ⬜ mixed HDL (vhdl doc §09) | ⬜ VHPI, `'foreign`, GHDL `vhpidirect` (rules.rs:6780 gap) | ⬜ | — | — | — | VHPI/foreign is not read |
| 40 | asm | — | — | — | — | — | — | — | 🟡 both directions at medium, but asymmetric underscore handling (asm→C strips one `_`, C→asm strips all; C `macho_fn` never reaches `_macho_fn`). Pass A also does not skip sites that already resolved intra-file, which gives duplicate edges (asm doc f1, f3; ffi.rs:39-40, 136) | — | — | — | — | — | — | Only the C family is bridged. Rust/Zig/Go/Fortran callers of asm labels are never linked (lang_family.rs:26-37) |
| 41 | smithy | — | — | — | — | — | — | — | — | — | — | — | — | 🟡 links, but the "service" is the namespace's last `.` segment, not the service shape (smithy doc fixture a), and links are not limited to operations (`structure Widget` → `AcmeStore.widget`, fixture f; descriptor.rs:104-106 filters only proto) | — | The owner key is wrong: a `WeatherService.GetCity` handler never links to `example.weather#GetCity` |
| 42 | proto | — | — | — | — | — | — | — | — | — | — | — | — | 🟡 rpc-only ✅ (descriptor.rs:104-106), but the owner match is a substring (`Foo` matches `Football`), fanout is uncapped (proto doc fixture c: 1 rpc → 6 methods), snake_case hosts never match (d1), and everything is always `low` | — | Rust/Ruby/Elixir/Erlang snake_case implementations never link (no `_` normalisation, descriptor.rs:85-89) |
| 43 | graphql | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ listed in `DESCRIPTOR_LANGUAGES` but never fires: qualified names have no owner and fields are not extracted (graphql.rs:111-113; graphql doc fixture t2) | — | SDL `Query.city` is never linked to its resolver, though `graph.rs:86-88` documents exactly that |
| 44 | openapi | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ not in `DESCRIPTOR_LANGUAGES` (descriptor.rs:47); `operationId` → handler is never linked (openapi doc fixture b) | — | `operationId: users.list_users` is never linked to `users.list_users` in Python |
| 45 | asyncapi | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ not in `DESCRIPTOR_LANGUAGES` (descriptor.rs:47) | — | Operations are never linked to handlers |
| 46 | lean | — | — | — | — | — | — | — | — | — | ⬜ `@[extern "c_fn"]` kept as the string `@[extern]`, argument dropped (lean.rs:265-268) | ⬜ `@[export]` does not classify. classify_ffi trims `#[`, `[` then `@`, leaving `[export` (ffi.rs:647-651), and `export` is not in `FFI_EXPORT_ATTRS` (roots.rs:173-183) | — | — | — | `@[extern]` does not link to C, and `@[export]` does not root |

## Parity gaps

The list is ranked by impact × how close a sibling language already is.

1. **The napi/UniFFI name twin has no snake_case → camelCase mapping.** Effort **S**.
   - Rust `#[napi] fn sum_values` is exposed to JS/TS as `sumValues`, and UniFFI exposes `add_nums` to Kotlin/Swift as `addNums`. Pass B indexes by the exact simple name (ffi.rs:188-200), so the default binding name never links.
   - Evidence: JS doc f7, TS doc f12, fixture d4 (`addNums` unlinked while `greet` linked). `grep camel ffi.rs` finds nothing.
   - Rust's own pyo3 and wasm-bindgen names are identity-mapped, so the gap is only in the renaming families.
2. **Descriptor matching does not normalise `_`, `-` or casing styles.** Effort **S**.
   - Idiomatic gRPC handlers in Rust (tonic `say_hello`), Ruby (`say_hello`), Elixir (`say_hello`), Erlang and Clojure (`say-hello`) can never match `SayHello`. Python, Java, Go, C#, Kotlin, Swift, Dart, Scala and PHP all link (d1).
   - Evidence: `by_method` is keyed on `simple_name.to_ascii_lowercase()` (descriptor.rs:85-89); fixture d1 (Ruby `say_hello` and Rust `say_hello` unlinked, CamelCase twins linked).
3. **JNI is classified on the Java side but never linked.** Effort **M**.
   - Kotlin `external` has no classification at all.
   - No plugin (C, C++ or Rust) records `JNIEXPORT` or `extern "system"`, so the `jniexport` arm of classify_ffi (ffi.rs:664) is dead code.
   - Java `native` is an Import (ffi.rs:668), and imports get no edge (ffi.rs:678-686).
   - Evidence: fixture d3 (`p.N.compute` attrs `['native']`, C `Java_p_N_compute` no attrs, no edge); kotlin doc f5; rust doc §09.
   - What a fix needs: map `Java_<pkg>_<Cls>_<m>` to `pkg.Cls.m`, and add the attribute to Kotlin `external`.
4. **The C/C++ plugins capture no attributes, unlike Rust and C#.** Effort **M**.
   - `__declspec(dllexport)`, `__attribute__((visibility("default")))`, `extern "C"`, `JNIEXPORT` and `EMSCRIPTEN_KEEPALIVE` are invisible.
   - As a result, C and C++ cannot be an FFI export side or a dead-code `ffi:export` root, while Rust `extern "C"` can (rust.rs:814).
   - Evidence: c.rs:216 and c.rs:247 (`attributes: Vec::new()`); cpp doc f9 (`extern "C" void c_export()` reported dead); fixture d3 (C `exported_c` has empty attributes).
5. **P/Invoke import → C body is never linked.** Effort **M**.
   - C# `[DllImport] add` stops at the C# stub. The in-tree C `add` therefore looks dead, even though the reverse direction (`UnmanagedCallersOnly` ← C) links.
   - F# `extern` declarations are not even callables, and `[LibraryImport]` is not classified.
   - Evidence: csharp doc f6; fsharp doc f8; ffi.rs:653-672, 678-686.
   - The same import-side gap covers Rust `extern "C" { fn f(); }` blocks, Zig `extern fn`, Fortran `bind(c)` interfaces, Haskell `foreign import`, OCaml `external`, Julia `ccall`, Swift bridging and Python ctypes. All of them have a declaration naming a C function in the tree, and none of them links it.
6. **wasm-bindgen and UniFFI are unrestricted, which produces phantom edges.** Effort **S**.
   - `allowed_source_languages` returns `None` for both (ffi.rs:700), so any same-named Kotlin/C/Python callable is drawn as a caller.
   - Evidence: fixture d4 (`kotlin wfun → rust wfun`, `c wfun → rust wfun` for a `#[wasm_bindgen]` export); rust doc ws3 (a Python stub linked to wasm-bindgen).
   - napi, pyo3 and c-abi were narrowed for exactly this defect (comment at ffi.rs:216-225). The fix is to restrict wasm-bindgen to JS/TS and UniFFI to Kotlin/Swift/Python/Ruby.
7. **The c-abi Pass B draws a C *definition* as a caller.** Effort **S**.
   - With both a C `exported_c(){…}` body and a Rust `#[no_mangle] exported_c`, Pass B emits `c exported_c → rust exported_c` (fixture d3).
   - It treats any same-named C callable as the caller, not only a prototype or a real call site (ffi.rs:200-262, `site_line: 0`).
8. **Path-qualified Rust FFI attributes are not linked.** Effort **S**.
   - `#[pyo3::pyfunction]` and `#[wasm_bindgen::prelude::wasm_bindgen]` mint `ffi-export` entries but get no Pass-B edge, because classify_ffi compares the full key (ffi.rs:647-653).
   - `deadcode/roots.rs::has_path_attr` already reads the last segment for test attributes (roots.rs:362-372).
   - Evidence: fixture d3 (`qualified_py` unlinked, `bare_py` linked).
9. **`#[pymethods]` members get no PyO3 edge.** Effort **S**.
   - The container attribute roots them (rules.rs:11776) but is not propagated to the members, so methods are never linked to their Python stubs (rust doc §09).
10. **GraphQL, OpenAPI and AsyncAPI descriptor links do not exist, though they are documented.** Effort **M/L**.
    - GraphQL is in `DESCRIPTOR_LANGUAGES` but cannot fire, because it has no owners and no field callables (graphql.rs:111-113).
    - OpenAPI and AsyncAPI are excluded (descriptor.rs:47), though the module comment (descriptor.rs:3-4) and README list them. OpenAPI `operationId` is a direct, exact name that is easier to link than proto.
11. **The Smithy descriptor owner is the namespace head, and links are not limited to operations.** Effort **S**.
    - Proto filters to `CallableKind::Method` (descriptor.rs:104-106); Smithy does not. The Smithy owner should be the `service`/`operation` binding, not the namespace's last segment (smithy doc fixtures a and f).
12. **Owner derivation fails for `:`-separated qualified names.** Effort **S**.
    - Lua `T:m` and Erlang `mod:f` produce no owner (names.rs:81-91), so their descriptor links never fire (d2, d3).
    - Conversely, Julia, Haskell and Clojure link only because a module or namespace stands in as the owner, the same "namespace head" accident as Smithy (d1, d2).
    - OCaml qualified names carry no module prefix at all (d2).
13. **Elixir descriptor matching uses only the last module segment.** Effort **S**.
    - `Hw.Greeter.Server.ping` does not link to `rpc Ping` and `Hw.GreeterServer.ping` does (d1 vs d2). This is the cause the elixir doc left unverified: `raw_owner` takes the last segment (names.rs:42-47).
14. **The asm bridge reaches only the C family.** Effort **M**.
    - The asm doc lists the duplicate-edge and underscore-asymmetry defects inside Pass A.
    - Rust `extern "C"`, Zig, Go (Plan 9 `TEXT ·f` asm) and Fortran callers never reach asm labels (`same_family` with `language_family("c")`, lang_family.rs:26-37).
15. **Markup code-behind is C#-only and produces roots, not edges.** Effort **L** per format.
    - Kivy kv produces real edges with confidence. C# XAML, Razor and tscn produce only `ui` framework entries (markup.rs:1-12).
    - Android XML (`android:onClick`) for Java/Kotlin, XIB/storyboard `IBAction` for Obj-C/Swift, Angular/Vue templates for TS/JS and Avalonia F# are not read (`markup_kinds_for`, rules.rs:11812-11819).
16. **Obj-C records no plain C calls.** Effort **M**.
    - Obj-C has the C family's inbound links (C → objc, asm → objc), but none outbound: objc → C, objc → asm, and objc → Rust c-abi (objc doc f4).
17. **Zig and Fortran exports are neither linked nor rooted, though the `zig-export` rule exists.** Effort **M**.
    - `zig-export` has `attributes: NONE` (rules.rs:6613-6628), and the Fortran plugin writes `attributes: Vec::new()` (fortran.rs:171). Both languages have first-class C-export syntax (`export fn`, `bind(c)`), comparable to Rust `#[no_mangle]`.
18. **The JVM and .NET sibling families are disabled.** Effort **L**.
    - Java ↔ Kotlin ↔ Scala ↔ Groovy ↔ Clojure and C# ↔ F# are commented out (lang_family.rs:32, 34). The C/C++/Obj-C family, by contrast, resolves across languages through includes.
19. **Native FFI for scripting and functional languages: none.** Effort **L** per language.
    - The missing forms: cgo, ctypes/cffi, `dart:ffi`/platform channels, LuaJIT `ffi.C`/`luaopen_*`, PHP `FFI::cdef`, Perl XS, Ruby `rb_define_method`, R `.Call`/Rcpp, Rustler/NIF, Haskell `foreign`, OCaml `external`, Julia `ccall`, Lean `@[extern]`, DPI-C, VHPI.
    - Every one of them names its C counterpart by a string literal, which the asm bridge shows is linkable.
20. **The build-language → source link is missing.** Effort **M**.
    - Starlark `srcs`, CMake `add_library` sources, Nix `builder`, Bash/PowerShell script invocation and Terraform → provider are not linked. None of the build languages has it.

## Doc corrections

- **dart.md §09** says "there is no Protobuf → Dart host link". **Wrong.** The descriptor linker is language-agnostic: fixture d1 drew `hello.Greeter.SayHello -> dart GreeterService::sayHello` (low, `descriptor-linker`).
- **swift.md §09** marks the descriptor link "Unverified for swift-grpc". **It links.** Fixture d1 drew `→ swift GreeterProvider.sayHello`.
- **swift.md §09** says "None of the `ffi::link_ffi` families covers Swift". **Incomplete.** UniFFI and wasm-bindgen are unrestricted (ffi.rs:700). Fixture d4 drew `swift greet → rust crate::greet` (uniffi, medium).
- **ruby.md §09** says "no Protobuf → Ruby rpc link". **Partly wrong.** A CamelCase `def SayHello` links (fixture d1). Only idiomatic `say_hello` fails.
- **groovy.md §09** says "no `groovy` in descriptor.rs", which implies no link. **It links** (fixture d1: `→ groovy GreeterImpl.sayHello`). The pass needs no per-language code.
- **perl.md §09** says "`descriptor.rs` has no Perl branch", which implies no link. **It links** (fixture d1: `→ perl GreeterService::SayHello`).
- **fsharp.md §09** says "no Protobuf/gRPC → F# link". **Wrong.** Fixture d1 drew `→ fsharp GreeterImpl.SayHello`.
- **objc.md §09** says "no objc host mapping … none found by grep". **It links** (fixture d1: `→ objc GreeterService::sayHello`).
- **julia.md §09** says "There are no descriptor links". **Wrong.** A module-level function links through its module name (fixture d1: `→ julia Greeter.sayhello`).
- **clojure.md §09** says "no Clojure host binding". **Wrong.** It links through the namespace head, the same accident as Smithy (fixture d1: `→ clojure greeter.service/SayHello`).
- **haskell.md §09** says "none for Haskell". **Wrong.** It links through the module name (fixture d2: `→ haskell GreeterImpl.ping`).
- **lua.md §09** says descriptor links are "not applicable". **They apply to `.` methods** (fixture d1: `→ lua GreeterService.SayHello`). Colon methods do not link (d2).
- **elixir.md §09** says "The cause of the `ping` miss is unverified". **Cause verified.** `raw_owner` takes the last module segment (`Server`), which does not contain `greeter`. `Hw.GreeterServer.ping` links (fixture d2).
- **erlang.md §09** says it is "Unverified whether a gpb-generated module would ever be linked". **No module-qualified Erlang function can link.** `greeter_impl:ping` has no derivable owner (names.rs:81-91; fixture d3).
- **scala.md §09** says "Whether `@native` (with `@`) classifies was not confirmed". **By code it does.** classify_ffi trims a leading `@` (ffi.rs:647-650), so `@native` → `("jni", Import)`, which gets no edge, as in Java.
- **java.md §09** says "Whether the C plugin records `JNIEXPORT` as an attribute is unverified". **It does not.** Fixture d3: `Java_p_N_compute` has no attributes, and c.rs:216/247 build `attributes: Vec::new()`.
- **lean.md §09** says "unverified whether `@[export]` matches `FFI_EXPORT_ATTRS`". **It does not.** `FFI_EXPORT_ATTRS` (roots.rs:173-183) has no `export`, and classify_ffi's trimming leaves `[export` (ffi.rs:647-651).
- **c.md §09** omits a defect. Pass B also draws a same-named C **definition** as a caller of a Rust c-abi export (fixture d3: `c exported_c → rust crate::exported_c` while both have bodies).
- **python.md and javascript.md §09** call wasm-bindgen "not applicable to Python" and imply it is JS-only. In fact wasm-bindgen is linkable from **any** language (rust doc ws3; fixture d4 Kotlin/C). Every row above marks that 🟡 as a phantom risk.
- **rust.md §09** is correct throughout. As a further note, `extern:C` (rust.rs:814) is classified by `classify_ffi` and is therefore a root (roots.rs:763-774), even though `FFI_EXPORT_ATTRS` (roots.rs:173) does not list it.
