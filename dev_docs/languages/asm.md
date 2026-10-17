# asm

The Assembly plugin (`AsmPlugin`, id `"asm"`, `crates/cgg-lang/src/plugins/asm.rs`, 275 lines) parses source with the crates.io grammar `tree-sitter-asm = "0.24"` (`Cargo.toml:122`, `crates/cgg-lang/Cargo.toml:61`; `tree_sitter_asm::LANGUAGE`, asm.rs:25). Nothing is vendored. The grammar produces a flat list of `meta` / `label` / `const` / `instruction` items (`~/.cargo/registry/src/*/tree-sitter-asm-0.24.0/grammar.js:16-48`). The plugin treats each `label:` as a callable and each instruction whose mnemonic is on a hard-coded list of call/jump opcodes (asm.rs:70-87) as a bare call site. There is no plugin-specific resolver kind: the `LanguagePlugin` trait has no `resolver_kind` method in this tree, and asm runs through the generic intra-file and cross-file steps. Its one asm-specific resolution path is the asm↔C/C++ bridge in `ffi::link_ffi` (Pass A, `crates/cgg-resolve/src/ffi.rs:36-169`). Registration is `reg.register(Box::new(asm::AsmPlugin))` (`crates/cgg-lang/src/plugins.rs:163`). The plugin ignores `ExtractCtx` (`_ctx`, asm.rs:30) and does not override `signals()`, so every `PluginSignals` bit is false (`crates/cgg-lang/src/lib.rs:304-306`, default).

Fixtures used below live under `<scratch>/parity/asm/{f1,f2,f3}`. They were run with `target/release/cgg` 0.11.0.

## 01. Detection & parsing

- **Extensions `.s`, `.S`, `.asm`** (asm.rs:21-23). Detection matches the extension case-sensitively first and then lowercased (`crates/cgg-lang/src/detect.rs:101-125`), so `.ASM` falls back to `.asm`. Fixture f1: `a.s` → `extension:.s`, `b.asm` → `extension:.asm`, `d.S` → `extension:.S` (audit `detected_via`). No other plugin claims `.s`/`.S`/`.asm` (grep over `crates/**/*.rs` for those literals finds only asm.rs).
- **Shebangs:** none (the trait default `&[]`, lib.rs `fn shebangs`).
- **Content detection:** none. The `.h` heuristic and the YAML/JSON sniff do not involve asm (detect.rs:78-125).
- **Grammar:** a single grammar, with no dialect switch (`ts_language_for` is not overridden). The one grammar covers GAS/AT&T, Intel/NASM-ish, ARM and RISC-V forms only loosely.
- **Preprocessed `.S`:** a `#` starts a line comment in the grammar (`grammar.js` `line_comment: seq('#', token.immediate(/.*/))`). So `#define` / `#include` in `.S` files are silently treated as comments. C-preprocessor macros such as `ENTRY(sym)` / `END(sym)` produce parse errors. Fixture f1 `d.S` has `parse_status: "error"`, and only `s_label` is extracted; `ENTRY(s_func)` yields no callable.
- **Re-parse / fallback:** none. A file that fails to parse is still walked, and labels that survive error recovery are kept (f1 `a.s` has `parse_status: "error"` but all 13 labels were extracted).
- **Skip conditions:** only the generic ones. `max_line_bytes` is not overridden (lib.rs default `None`). The generic depth guard `MAX_TREE_DEPTH = 4000` applies (`crates/cgg/src/lib.rs:155,451`), but a flat grammar never gets close to it. The minified-file rule only concerns `FileFacts::locals`, which asm never fills.
- **Not supported:** `.inc`, `.nasm`, `.x86` and similar extensions are not detected (they are absent from asm.rs:22). No test covers asm detection: `detect.rs::each_v1_extension_detects` (detect.rs:674-690) lists no asm case.

## 02. Callables extracted

- **Plain label** `foo:` → `DefRecord` with `simple_name = qualified_name = "foo"`, `DefVariant::FreeFunction` (kind `function`), `attributes: ["label"]`, `visibility: ""`, and `signature_hint` = the label text (asm.rs:122-148). Test: `labels_are_callables` (asm.rs:232-243).
- **Body span fix-up:** the grammar gives a `label` node only the `name:` token. `extract` stretches each def's range up to one byte before the next label, or to EOF (asm.rs:42-63). Every label is therefore a "function" running to the next label, whatever the label's role is.
- **Local labels** `.Lloop:` (the grammar's `_ident` aliased as `ident`) **are callables**. They split the enclosing routine in two: in fixture f1, `_start` ends at line 7 and `.Lloop` (lines 7-19) takes ownership of every call after it. There is no fall-through edge from `_start` to `.Lloop`.
- **Numeric local labels** `1:` are callables named `"1"` (fixture f1, callable `('1', 38, 40)`).
- **Qualified names / owners:** none. The qualified name equals the simple name. There is no section, file or `.globl` qualification, and no owner.
- **Not supported:**
  - Labels that match the grammar's `meta_ident` regex `/\.[a-z_]+/` (e.g. `.loop:` all lowercase). `record_label` only looks for an `ident` child (asm.rs:123-127), and these labels have a `meta_ident` child. Fixture f3: `.loop:` gives no callable, and its `call target_b` is unattributed (`no-enclosing-callable`).
  - The keyword form `label kw_label`: the grammar's field is `word`, not `ident`. Fixture f3 shows no callable.
  - `ENTRY(x)`, `SYM_FUNC_START(x)`, `PROC`/`ENDP`, `.macro` definitions, and `.type foo,@function` / `.size` (the latter are `meta` items and are ignored, asm.rs:99-120). Fixtures f1 `d.S` and f2.
  - `const NAME value` items are not callables.
  - Nested defs, methods, constructors, closures, properties, operators, overloads and abstract members are not applicable to assembly.
  - **Duplicate labels in one file** become two callables with the same name. A call to that name is refused as `ambiguous-in-file` (fixture f3: `call dup` → `U dup 18 ambiguous-in-file`).

## 03. Call-site forms

- **Bare call/jump to a label**, e.g. `call foo` / `jmp foo` / `bl foo` / `jal foo`. When the first child is a `word` mnemonic on the list (asm.rs:150-165), the first operand is taken as the target: the first `ident` operand whose leaf `word` text does not start with `%`/`$` (asm.rs:167-181). The site is recorded as `RefRecord { name, receiver_hint: "" }` (asm.rs:183-189). Tests: `call_and_jmp_captured`, `arm_bl_captured`, `registers_are_not_calls` (asm.rs:245-274).
- **Mnemonic list** (asm.rs:71-86), matched case-insensitively (fixture f3: `BL target_a` and `CALL target_b` both bound):
  - x86: `call callq calll callw jmp jmpq jmpl je jne jz jnz jg jge jl jle ja jae jb jbe jo jno js jns jc jnc jp jnp loop`
  - ARM: `bl blx bx b b.eq b.ne b.lt b.gt b.le b.ge`
  - RISC-V: `jal jalr`
  - MIPS/PPC: `bal bgez bgezal bltz bltzal jr`
- **PLT suffix** `call printf@PLT` → name `printf` (fixture f1 external record `printf`).
- **Every site is an unqualified bare name.** There are no receivers, paths, chains, constructors, generics, operators, paren-less marks or arity (`RefRecord::arity` is never set).
- **Not supported or wrong:**
  - **Dot-prefixed targets** (`jmp .Lloop`): the operand is an `_ident` leaf with no `word` inside, so `find_word_leaf` returns `None` (asm.rs:192-209) and nothing is recorded. Fixture f2 shows no `foo → .Lloop` edge.
  - **ARM conditional suffix branches** `b.eq`, `b.ne` and width suffixes such as `bl.w`: the grammar's `word` is `/[a-zA-Z0-9_]+/`, so the mnemonic is never the string `b.eq`, and the `b.eq` … arms in the list can never match. Fixture f2: `b.eq cond_fn` gives no edge. Fixture f3: `bl.w target_a` gives no edge.
  - **Unlisted branch opcodes**: RISC-V `beq/bne/blt/bge`, `tail`, `call` pseudo with `j`; AArch64 `cbz/cbnz/tbz/tbnz/br/blr`; x86 `jecxz`, `jrcxz`, `loope/loopne`, `syscall`; MIPS `beq/bne/j/jalr`; PPC `bctr/bcl`. Fixture f1: `beq a0, a1, rv_branch`, `cbz x0, cbz_target` and `tail tail_target` produced no edges.
  - **Indirect calls** `call *%rax`: the leaf `word` is `rax`, without the `%`, so the register check at asm.rs:175 does not fire. A bogus site named `rax` is recorded (fixture f1: external record `rax`; with `--include-external`, an `<external>::rax` node). The `$` check is likewise unreachable for `word` leaves. `call *(%rbx)` records nothing.
  - **Offset / pointer operands** `call target_a+4`, `call qword ptr [x]`, `call [rel x]` are not recorded (fixture f3: no edge or record at lines 9-11).
  - **Numeric local refs** `jmp 1b` / `1f` are recorded as the literal name `1b` and never bind to label `1` (fixture f1: external `1b`).
  - **Value references** are never recorded: address-taking (`ldr x0, =fn`, `adr x1, fn`, `lea fn(%rip)`) and data tables (`.quad fn`, `.long fn`, `dd fn`). Fixture f3 shows no record. The plugin never sets `RefRecord::value_ref`.
  - **Macro invocations** (`MYMAC` from `.macro`): not expanded or recorded. A `call` inside a `.macro` body is attributed to whichever label precedes it, or is unattributed (fixture f1 `d.S`: `helper` at line 4 has `src: None`).
  - Registrar-argument capture: not applicable / none.

## 04. Imports & module resolution

- **None.** The plugin writes no `ImportRecord`s. `.include "file.s"` is a `meta` item and is ignored (fixture f3 `.include "other.s"` produced no import).
- **Cross-file asm → asm calls never resolve.** The only by-name cross-file path for an import-less language is step 1d, and it is gated on `has_imports` (`crates/cgg-resolve/src/cross_file.rs:6015-6021`). An asm file has no imports, so a call to a label in another `.s` file is recorded as `candidates-in-other-files` and gets no edge. Fixture f2: `x.s local_l: call bar` → `U bar … candidates-in-other-files {candidates: 1}`, and `y.s bar: call baz` → same; the graph has 1 edge, 0 cross-file. CHANGELOG.md:5470-5476 also lists Assembly among the languages that "genuinely yield none" cross-file, and the README table has `—` in the cross-file column (README.md:524).
- **Cross-language only:** asm ↔ C/C++/ObjC through `ffi::link_ffi` (see §09).
- `FileFacts::packages`, manifests, `heads.rs` external-module refusal, `modules.rs` and `not-in-imported-module` do not apply (no import table). Names that no callable carries are filed as external with `name_screen_applied: "external"` (fixture f1 `rax`, `printf`, `1b`, `_c_under`).
- The C-family include resolver treats `<asm/…>` and `<asm-generic/…>` as platform headers (`cross_file.rs:3207-3214`, `is_platform_header`). That is about Linux C headers, not the asm language.

## 05. Type inference & receiver typing

- Not applicable: assembly has no types, receivers, locals or parameters. `type_hints`, `declared_type`, closure typing, implicit receivers and weak receiver types never see asm data, because the plugin records no `arg_type`, `local_types`, `scoped_types` or receivers (asm.rs:183-189 sets only `name`, `receiver_hint: ""`, line and byte).

## 06. Inheritance, dispatch & overloading

- Not applicable. There are no class records, bases, super calls, trait impls or extension methods. asm is absent from `OVERLOAD_ARITY_LANGS` and `INHERITANCE_LANGS` (grep `"asm"` in `crates/cgg-resolve/src` hits only ffi.rs and deadcode/roots.rs). `--dynamic-dispatch` adds nothing. The dead-code capability table reports `dispatch: none` (fixture f2 `.deadcode.json` capabilities).

## 07. Scoping & shadowing

- **None.** No `FileFacts::locals` and no `shadowed-by-local` (the plugin never touches locals). Labels do not nest, so there is no lexical narrowing and no positional shadowing.
- **Same-name labels in one file** are refused as `ambiguous-in-file` (fixture f3). In different files, each file's call binds to its own copy intra-file (fixture f3: `n.s other_file_fn → dup` at `high`).
- **Conditional assembly** (`.if` / `.ifdef` / `#ifdef`) is not modelled. Every label in every branch is a callable. Unverified beyond the grammar: `.if` parses as a `meta` item, which the plugin ignores.

## 08. Macros, preprocessor & codegen

- **None.** `.macro`/`.endm`, `%macro` (NASM), `#define` and `#include` in `.S` are not expanded. `#…` lines are comments in the grammar (see §01). There is no `macro:<name>` tagging and no `name_mentions`. `c_macro.rs` expansion is wired only for the C and C++ plugins (CLAUDE.md architecture; asm.rs never references it).
- **Consequence:** kernel- and libc-style `.S` that defines symbols through macros (`ENTRY(memcpy)`, `SYM_FUNC_START`) yields few or no callables, and parse errors (fixture f1 `d.S`).

## 09. FFI & cross-language links

- **asm → C/C++/ObjC** (Pass A, `ffi.rs:93-125`). For every asm reference, the linker looks up C-family callables with that simple name. Only when none exists does it retry with **one** leading `_` stripped (the Mach-O / 32-bit Windows prefix, `ffi.rs:58-80`; CHANGELOG.md:222-224). It then emits `Via::Ffi("asm-c")` at **medium** from the enclosing label (`enclosing_callable`, ffi.rs:606-622). Tests: `an_asm_call_strips_at_most_one_underscore_and_only_when_needed` (ffi.rs:961-995). Fixture f1: `nasm_entry → c_func` and `→ c_under` (from `call _c_under`), shown as `?ffi`.
  - **Gap:** Pass A does not check whether the site already resolved intra-file. `call helper`, with `helper:` in the same `.s` and an `extern void helper(void);` prototype in a `.c` file, produces both the intra-file edge to the asm label and an `asm-c` FFI edge to the C prototype (fixture f1: `_start --> N3` and `_start -->|?ffi| N17`). The module comment says "that doesn't resolve intra-file" (ffi.rs:39-40), but the code has no such check.
- **C/C++/ObjC → asm** (ffi.rs:126-168). For C refs that are not value or string references (ffi.rs:130-135; test `a_value_reference_to_an_asm_label_is_not_an_ffi_call`, ffi.rs:928-958), the linker matches asm labels named either as written or with **all** leading underscores trimmed (`trim_start_matches('_')`, ffi.rs:136). It emits `Via::Ffi("c-asm")` at medium. Fixture f1: `cmain → helper`, `→ other` (from `_other()`) and `→ unused` (asm).
  - **Asymmetry gap:** the direction that adds a prefix is missing. C `macho_fn()` never reaches the asm label `_macho_fn` (fixture f3: only the intra-file edge to the C prototype). The C→asm direction also strips every leading `_`, while asm→C strips at most one.
- Language family: `language_family("c")` = `c`, `cpp`, `objc` (`crates/cgg-core/src/lang_family.rs:26-37`). Rust `extern "C"`, Zig, Go and other callers are never linked to asm labels.
- PyO3, wasm-bindgen, napi, JNI, P/Invoke, descriptor links and markup code-behind: not applicable.

## 10. Frameworks & entry points

- **No framework rules apply to asm.** A grep for `asm` in `crates/cgg-core/src/frameworks/rules.rs` hits only `wasm_bindgen` (line 1457) and `regasm` prose (line 9376). No `RuleSpec` names the language, and there are no member_sources, signature_methods, container_attributes, markup_kinds or registrar verbs.
- Framework coverage reports asm under `no_markers` (fixture f1 audit: `"no_markers": [{"language": "asm", "files": 3}]`).
- `crates/cgg/tests/detect_prefixes.rs:92` has an asm `import_form` (`.global _start\n_start:\n  call {prefix}`), but with no asm rules it exercises nothing.

## 11. Test harness entries

- None. There are no test/benchmark/fuzz rules for asm, no test-file gating, and asm is not in `TEST_CLASS_MEMBERS_ARE_ROOTS`. The capability table reports `test_tagging: none` (fixture f2).

## 12. Dead-code analysis

- **Roots:** `main` (`UNIVERSAL_ENTRY`, `crates/cgg-resolve/src/deadcode/roots.rs:19`) and `_start` (`LANG_ENTRY ("asm", &["_start"])`, roots.rs:30). Fixture f1: roots `_start` and `main`, both `program-entry`, rule `builtin:main`.
- **`.globl` / `.global` / `global` exports are not roots.** The plugin leaves `visibility` empty (asm.rs:144). Fixture f2: `.globl foo` gives `roots: []`, and `foo` is reported `never-referenced`.
- **Capabilities** (fixture f2 `.deadcode.json`): `class: degraded`, `visibility: none`, `attributes: full` (only because every def carries the `"label"` attribute, observed in `caps.rs:132-133`), `value_references: none`, `dispatch: none`, `exports: none`, `test_tagging: none`, `max_confidence: medium`. The class is `Degraded` because the visibility signal is absent (`caps.rs:165-172`). asm is in neither `SCRIPT_DRIVEN` nor `DESCRIPTOR` (caps.rs:27-41), and has no `BLIND_SPOTS` entry (caps.rs:44-67).
- **Findings cap at medium**, with evidence `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges` (fixture f2). The default `--dead-code` report shows 0 high findings for asm, and the rest are withheld (f1: "0 … high … 9 withheld"; f2: 7 withheld).
- **Structural false positives:**
  - Local labels (`.Lloop`) and numeric labels (`1`) are separate callables.
  - Fall-through into the next label is no edge.
  - `jmp .Lxxx` is not captured.
  - Labels reached only by address (jump tables, `.quad fn`, `ldr =fn`) have no value refs.
  - Every one of these appears unreferenced (fixture f2: `.Lloop`, `cond_fn` and `wide` are all findings).
- No unreachable-code or reflection doubt for asm (all signals off).

## 13. Builtins & stdlib tables

- **No `stdlib/asm.txt`.** `crates/cgg-core/src/stdlib/` has 30 tables and none is for asm. `stdlib_names` (`crates/cgg-core/src/stdlib.rs:221-250`) has no `"asm"` arm, so `stdlib_names("asm")` is `None`.
- There is no rust_types-like table, and asm has no entries in `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` (not applicable).
- `--include-stdlib` mints nothing for asm (fixture f1: `0 stdlib`). `--include-external` mints `<external>::<name>` for every unbound target, including the bogus register `rax` and `1b` (fixture f1: `<external>::rax`, `<external>::printf`, `<external>::1b`).

## 14. Confidence & certainty

- **Intra-file label binding is `high`** (`resolver: intra-file`, fixture f2: `foo → local_l high`; f3: four `high` intra-file edges). Mermaid draws these without a `?`.
- **Every FFI edge (asm-c, c-asm) is `medium`** (ffi.rs:117,160) and renders as `?ffi` in mermaid (fixture f1).
- **Same-file duplicate labels** give no edge at all (`ambiguous-in-file`).
- There is no `low` path for asm. `--guess-unique-names` is Rust-only (CLAUDE.md) and does not rescue asm cross-file calls.

## 15. Dynamic use & reflection

- None. asm is not covered by `dynuse` (Python, JS, Java, C#, Ruby and PHP only, per CLAUDE.md). The plugin sets no dyn-use signals.
- Register-indirect calls (`call *%rax`, `blr x8`, `jalr t0`) are unmodelled: the `%`-prefixed form becomes a bogus site named after the register (§03), and the others are unrecorded or unlisted.
- Jump tables and computed `jmp *table(,%rax,8)` produce no edges.

## 16. Known gaps & limitations

- **No asm → asm cross-file resolution.** Step 1d requires imports (`cross_file.rs:6015-6021`), and asm never records any, not even `.include`. Fixture f2 shows every inter-file call as `candidates-in-other-files`. CHANGELOG.md:5470-5476 and README.md:524 (`—`) state this too.
- **Dot-prefixed jump targets are dropped** (`jmp .Lloop`). `find_word_leaf` requires a `word` leaf (asm.rs:173,192-209). Fixture f2.
- **The ARM `b.cond` arms are dead code.** The grammar's `word` cannot contain `.` (asm.rs:81 vs `grammar.js word: /[a-zA-Z0-9_]+/`). Width suffixes such as `bl.w` fail the same way. Fixtures f2 and f3.
- **The opcode list misses common branches:** AArch64 `cbz/cbnz/tbz/tbnz/br/blr`, RISC-V `beq/bne/blt/bge/j/tail`, MIPS `beq/bne/j`, x86 `jecxz/loope/loopne` and others (asm.rs:71-86). Fixture f1.
- **The register filter is ineffective.** `%`/`$` are not part of the `word` leaf (asm.rs:175), so `call *%rax` records `rax` (fixture f1).
- **Labels do not equal functions.** Local `.L`/numeric labels split routines (no fall-through edge), lowercase `.name:` labels (`meta_ident`) and `label name` labels are dropped, and `ENTRY()`/`.macro`/`.type @function` are ignored (§02).
- **`.S` preprocessing:** `#…` lines are comments, and C macros cause parse errors (fixture f1 `d.S`).
- **No visibility from `.globl`**, so exported symbols are dead-code candidates and the language is `Degraded` with a medium cap (§12).
- **FFI asymmetries:** asm→C does not skip sites that already bound intra-file (duplicate edge to a C prototype, fixture f1). C→asm never adds the `_` prefix (`foo()` → `_foo:` is missed, fixture f3) and strips every leading `_` (ffi.rs:136).
- **Duplicate labels in one file** refuse every call to that name (`ambiguous-in-file`).
- **Test coverage is thin:** 4 unit tests in asm.rs (`labels_are_callables`, `call_and_jmp_captured`, `registers_are_not_calls`, `arm_bl_captured`) and 2 asm FFI tests in ffi.rs. There is no integration fixture and no detection test. Benchmark entries are `asm-xv6` and `asm-xv6-mixed` (`scripts/benchmark.sh:99-100`); README.md:1351 reports xv6 asm-only at 22 callables, 4 edges, 0% cross-file.
- No `#[ignore]` tests for asm were found (grep in asm.rs and in ffi.rs's asm tests).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.s`/`.S`/`.asm` by extension; one loose grammar; `#` is a comment, so `.S` preprocessing is lost |
| 02 Callables extracted | partial | every `ident` label is a function to the next label; `.lower:`, `label x`, ENTRY()/macros missed |
| 03 Call-site forms | partial | listed call/jump mnemonics with a word operand only; `.L` targets, `b.cond`, many branches, indirect and value refs missed |
| 04 Imports & module resolution | none | no imports; asm→asm cross-file never resolves (`candidates-in-other-files`) |
| 05 Type inference & receiver typing | n/a | untyped language; no receivers |
| 06 Inheritance, dispatch & overloading | n/a | no classes, overloads or dispatch |
| 07 Scoping & shadowing | none | no locals; duplicate same-file labels give `ambiguous-in-file` |
| 08 Macros, preprocessor & codegen | none | `.macro`, `%macro`, `#define`, `ENTRY()` not expanded |
| 09 FFI & cross-language links | partial | asm↔C/C++/ObjC by name at medium; underscore-prefix asymmetry; duplicate edge to C prototypes |
| 10 Frameworks & entry points | none | no rules mention asm |
| 11 Test harness entries | none | no test rules |
| 12 Dead-code analysis | minimal | roots `main`/`_start` only; Degraded, medium cap; `.globl` ignored; label-splitting false positives |
| 13 Builtins & stdlib tables | none | no `stdlib/asm.txt`; externals include bogus register names |
| 14 Confidence & certainty | partial | intra-file `high`; all FFI `medium` (`?ffi`); nothing cross-file |
| 15 Dynamic use & reflection | none | indirect and computed jumps unmodelled |
| 16 Known gaps & limitations | n/a | see list above (cross-file, `.L` targets, `b.cond`, `.globl`, macros) |
