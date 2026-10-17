# vhdl

The VHDL plugin (`VhdlPlugin`, `crates/cgg-lang/src/plugins/vhdl.rs`, 322 lines including three inline tests) parses `.vhd` and `.vhdl` files with the crates.io grammar `tree-sitter-vhdl` 1.4 (`Cargo.toml:121` `tree-sitter-vhdl = "1.4"`, `crates/cgg-lang/Cargo.toml:60`; `tree_sitter_vhdl::LANGUAGE`, vhdl.rs:25-27). Nothing is vendored. It is registered at `crates/cgg-lang/src/plugins.rs:162`. The plugin does four things:

- extracts `entity_declaration`, `architecture_definition` and `subprogram_definition` as callables;
- records `library_clause` / `use_clause` as imports;
- records one reference per architecture, for its `of <entity>` clause;
- records **no call sites**. The module doc says call sites are captured "best-effort via `function_call_expression` if present" (vhdl.rs:6-8), but no code does this. The grammar's call node kinds are `function_call`, `procedure_call_statement` and `concurrent_procedure_call_statement` (checked in `tree-sitter-vhdl-1.4.0/src/node-types.json`), and the walker ignores all three.

The `LanguagePlugin` trait has no `resolver_kind` method in this tree: `grep -rn resolver_kind crates` returns nothing, although CLAUDE.md still names one. So there is no per-language resolver kind. VHDL goes through the generic resolver path: `intra_file::link_file`, then `cross_file::resolve`.

The plugin does not override any of these, so all keep their trait defaults:

- `signals()`: no dead-code signals are declared;
- `shebangs()`;
- `max_line_bytes()`;
- `ts_language_for()`.

Fixtures cited below live under `<scratch>/parity/vhdl/`:

- `a/`: `pkg.vhd` and `top.vhd`, with packages, a protected type, overloads, a process, instantiations and a configuration;
- `b/`: cross-file, case and duplicate tests;
- `c/`: cross-file binding with and without `use`.

Each fixture was run with `target/release/cgg` 0.11.0.

## 01. Detection & parsing

- **Extensions `.vhd` and `.vhdl`** (vhdl.rs:22-24).
  - Fixture b: `ent.vhdl` gives `detected_via: "extension:.vhdl"`, and `arch.vhd` gives `"extension:.vhd"`.
  - **Upper-case `d1.VHD` is detected** as `extension:.vhd`, so extension matching ignores case (fixture b).
- **Shebangs**: none. `shebangs()` is not overridden, and the default returns `&[]` (`crates/cgg-lang/src/lib.rs:298-300`).
- **Content detection**: none. `crates/cgg-lang/src/detect.rs` has no VHDL branch (`grep -n -i vhd detect.rs` returns nothing).
- **Grammar**: one grammar for every file. `ts_language_for` is not overridden (lib.rs:322-324). VHDL-2008 constructs parsed in fixture a with `parse_status: ok`:
  - protected types and bodies;
  - `impure function`;
  - `function "+"`;
  - `entity work.x` instantiation;
  - configuration declarations.
- **Re-parse / fallback**: none. Unlike C#'s `#if` blanking, the plugin has no error-recovery re-parse.
- **Skip conditions** come from the generic driver only:
  - `cgg_lang::exceeds_depth` against `MAX_TREE_DEPTH = 4000`, which gives `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155`, `:451-454`);
  - the generic long-line / minified handling (`lib.rs:157`, `:439`).
  - VHDL declares no `max_line_bytes`; the default is `None` (lib.rs:308-316).
  - The walker recurses (`walk` → `walk_children`, vhdl.rs:80-185), which is why the depth gate matters.
- **Framework detect prefixes**: `crates/cgg/tests/detect_prefixes.rs:90` synthesizes `library {prefix};\nuse {prefix}.all;` in a `.vhd` file to prove that the `vhdl-toplevel` detect prefixes fire.

## 02. Callables extracted

- **Entity**: `entity counter is port(...); end entity;` becomes a `FreeFunction` with the attribute `entity` (`record_named`, vhdl.rs:82-86, 187-212). The QN is the bare name. Fixture a: `counter`, `tb`.
- **Architecture**: `architecture rtl of counter is ... end architecture;` becomes a `FreeFunction` with the attribute `architecture` (vhdl.rs:87-129).
  - The architecture name is pushed as a scope, so what it contains is qualified `rtl.<name>`.
  - Fixture a: `rtl`, `sim`. Test `plugins::vhdl::tests::entity_and_architecture` (vhdl.rs:276-292).
- **Subprogram bodies** (`subprogram_definition` with a `procedure_specification` or `function_specification` child) are recorded with the attribute `function` or `procedure` (vhdl.rs:130-134, 214-253).
  - The variant is `FreeFunction` at file or package scope, and `InherentMethod` inside an architecture.
  - Test `function_and_procedure` (vhdl.rs:294-305). Fixture a: `rtl.local_dbl` (inherentmethod).
- **Subprogram nested in a process**: `procedure inner` declared in a process's declarative part is qualified as `rtl.inner`. The process itself is not a scope (fixture a). A subprogram nested in another subprogram is also qualified by the architecture only, because no scope is pushed for subprograms (vhdl.rs:130-134).
- **Package body members**: `package body util_pkg is function inc ...` gives `inc` with **no package qualifier**. `package_definition` and `package_definition_body` push no scope (fixture a: `inc`, `inc`, `log_msg` are all bare `freefunction`s).
- **Protected type body members**: `procedure bump` and `impure function get` inside `type counter_t is protected body` give bare `bump` and `get` (`freefunction`), with no owner (fixture a).
- **Overloads**: both `inc(integer)` and `inc(unsigned)` become separate callables with the same QN (fixture a, ids `Cpuaj70lr0e` and `C17g0xukc8so`). Nothing groups them as an overload set (see 06).
- **Signature hint**: `super::extract_signature(text)` of the whole node (vhdl.rs:106, 207, 244). For an architecture this is its whole header and declarative region (fixture a, the dead-code JSON `signature_hint` for `rtl`).
- **Visibility**: always the empty string (vhdl.rs:107, 208, 245).

Not supported:

- **Subprogram declarations** (`subprogram_declaration`, for example the package-spec prototype `function inc(a : integer) return integer;`) are not extracted. The protected-type declaration members are not extracted either. Only bodies are (fixture a: 5 callables from pkg.vhd, all bodies).
- **Operator functions**: `function "+"(l, r : bit) return bit` is not extracted. The designator is an `operator_symbol`, not an `identifier`, so `record_subprogram` returns early (vhdl.rs:220-226). Fixture a: no `+` callable.
- **Processes** (`process_statement`, labelled or not) are not callables. This is also stated in the `vhdl-toplevel` gap string (rules.rs:6780).
- **Packages / package bodies** are not callables or scopes (fixture a).
- **Configurations** (`configuration cfg of tb`) are not extracted (fixture a).
- **Component declarations, blocks, generate statements, generic subprograms and `subprogram_instantiation_declaration`** are not extracted (no match arm in vhdl.rs:81-171).
- **Case**: VHDL identifiers are case-insensitive, but QNs keep the source spelling (fixture b: `Adder`). See 04 for the consequence.

## 03. Call-site forms

- **Architecture → entity binding**: `architecture rtl of counter` records a `RefRecord { name: "counter" }` at the architecture's start line and byte (vhdl.rs:111-123). It is the only reference the plugin ever emits.
  - Same-file: fixture a `rtl --> counter` and `sim --> tb`, both `high`, resolver `intra-file`.
  - A selected name (`of work.e1`) records `name = "work.e1"` verbatim, which lands in the `external_calls` bucket (fixture c, r2.vhd).

Not supported (the grammar has nodes for these, but the plugin does not read them):

- **Function calls**: `c <= inc(c);` (`function_call`). Fixture a: no edge, and the audit has no unresolved record at all (0 unresolved).
- **Procedure calls**: `log_msg("tick");` and the selected form `work.util_pkg.log_msg("q");` (`procedure_call_statement`). The bare paren-less call `inner;` is not recorded either.
- **Concurrent procedure calls** (`concurrent_procedure_call_statement`).
- **Entity instantiation** `dut: entity work.counter port map(...)`, and **component instantiation** `dut2: counter port map(...)` (`component_instantiation_statement`). Fixture a: no `sim → counter` edge.
- **Configuration bindings** (`for sim ... use entity ...`).
- **Operator calls** (`a + 1` resolving to an overloaded `"+"`).
- **Value refs, callbacks, registrar arguments, arity recording**: none. `extract` ignores `_ctx` (vhdl.rs:31), so `reference_sites` and user registrar verbs have no effect. No `RefRecord::arity` is set.
- README `## Benchmark` row: `| UVVM | vhdl | 1,036 | 0 | — | 162ms |` (README.md:1350). That is 1,036 callables and **0 edges** on the benchmark repo, which matches the absence of call-site extraction.

## 04. Imports & module resolution

- **`library ieee, work;`** gives one `ImportRecord { kind: "library", path: <name> }` per name in the `logical_name_list`, split on `,` (vhdl.rs:135-154). Test `library_and_use_are_imports` (vhdl.rs:307-321).
- **`use ieee.std_logic_1164.all;`** gives `ImportRecord { kind: "use", path: "ieee.std_logic_1164.all" }`, with the whole `selected_name_list` text verbatim (vhdl.rs:155-169).
  - A multi-name `use a.b, c.d;` becomes one record with the comma-joined text, which is not split (vhdl.rs:157; unverified by fixture).
- **Effect in resolution**: the generic `"use"` arm of `cross_file::resolve` pushes the path onto `unqualified_prefixes` (`crates/cgg-resolve/src/cross_file.rs:1940-1955`). This is what lets an architecture's `of <entity>` bind to an entity in **another file**.
  - With `use work.all;`, fixture b binds `beh2 -> Adder` at `medium`, resolver `cross-file:imports`.
  - Without any `use`, fixture c's `r of e1` (entity in e.vhd) is **refused** with `candidates-in-other-files {candidates: 1}`.
- **Dead branch**: `cross_file.rs:2124-2129` lists `lang == "vhdl"` in the `"include" | "add_subdirectory" | "find_package"` arm, but the VHDL plugin never emits those kinds. The arm is unreachable for VHDL.
- **Not supported**:
  - **No library / design-unit model**: `work.util_pkg` is never mapped to the file that declares `package util_pkg`.
  - No `FileFacts::packages` records.
  - No manifest reading (no `.qsf`, `.xpr`, `vunit run.py`, or `Makefile` source lists).
  - No `heads.rs` external-module refusal: VHDL is not in the `WorkspaceVocab` language set (`grep -n -i vhdl crates/cgg-resolve/src/heads.rs` is empty).
  - No `modules.rs` / `libscope.rs` handling. The `"use"` and `"library"` arms in libscope.rs:320 and :1051 are for Dart and Elixir.
  - No `not-in-imported-module`.
  - **Case-sensitive binding**: `architecture beh of adder` against `entity Adder` does not bind, and goes to the `external_calls` bucket with `no-candidate-in-file` (fixture b, `b.json.audit.json`). In VHDL the two names are the same.
  - **Duplicate entities**: two `entity twin` in different directories give `candidates-in-other-files {candidates: 2}`, which is refused (fixture b).

## 05. Type inference & receiver typing

Not applicable in practice. The plugin records no call sites with receivers, so type propagation has nothing to type. It records no `locals`, `local_types`, `scoped_types`, parameter types or return types. The vhdl.rs struct-literal `RefRecord` sets only `name`, `receiver_hint` (empty), `site_line` and `site_byte` (vhdl.rs:115-121).

None of the following is supported:

- VHDL signal, variable or port types;
- protected-type method calls (`counter.bump`);
- record field typing;
- `declared_type` wrapper handling.

## 06. Inheritance, dispatch & overloading

- VHDL has no class inheritance. The nearest equivalents are listed below, and none of them is modelled:
  - entity/architecture pairing, which is modelled only as a reference (03);
  - protected types;
  - configurations.
- **Overloading**: VHDL resolves subprogram overloads by parameter and result type. cgg extracts both overloads (fixture a, two `inc`), but VHDL is not in `OVERLOAD_ARITY_LANGS` and no arity is recorded, so there is no overload narrowing. With no call sites, it never matters.
- **Multiple architectures of one entity**: `beh` and `beh2` of `Adder` each add a separate edge to the entity, when they bind at all (fixture b). Nothing models which architecture a configuration selects.
- **Not in `INHERITANCE_LANGS`**, so `dispatch::fanout` and `inheritance_fanout` do nothing for VHDL. There is no `trait_impl_target`, no `member_of_calling_class`, no `overrides_below` and no `super`. Evidence:
  - `grep -rn -i vhdl crates/cgg-resolve/src` returns only cross_file.rs:2128;
  - the dead-code capabilities row reports `dispatch: none` (fixture a, `dc.json`).

## 07. Scoping & shadowing

- **Scope stack**: only `architecture_definition` pushes a scope (vhdl.rs:124-126). Entities, packages, package bodies, protected bodies, processes, blocks and subprograms do not.
- **Intra-file binding** of the one reference kind works: `rtl --> counter` is `high`, resolver `intra-file` (fixture a).
- **Not supported**:
  - no `FileFacts::locals` (VHDL is not handled in `crates/cgg-lang/src/locals.rs`; `grep -i vhdl` is empty), so there is no `shadowed-by-local`;
  - no lexical narrowing of nested definitions (VHDL is not in that list in CLAUDE.md or the code);
  - no positional shadowing;
  - no `DefRecord::conditional` (`if generate` / `case generate` bodies are not distinguished).

## 08. Macros, preprocessor & codegen

- VHDL has no macro system.
- **VHDL-2019 conditional analysis** (`` `if `` / `` `else `` / `` `end `` tool directives) is not handled. It is unverified whether the 1.4 grammar parses it.
- **Generic packages and subprogram instantiation** (`package p is new gen_pkg generic map(...)`, `function f is new g`) are not expanded or extracted (no match arm in vhdl.rs). No `macro:<name>` tagging, no `name_mentions`.

## 09. FFI & cross-language links

None.

- `ffi::link_ffi` has no VHDL arm. `grep -i vhdl crates/cgg-resolve/src/ffi.rs` is empty.
- VHPI, the `'foreign` attribute and the GHDL `vhpidirect` C bindings are not read. The `vhdl-toplevel` gap string names "a VHPI/foreign attribute" as an unhandled entry form (rules.rs:6780).
- There are no descriptor links and no markup code-behind.
- Mixed VHDL/Verilog instantiation across languages is not linked.

## 10. Frameworks & entry points

- **`vhdl-toplevel`**: `kind: TrustKind::Lifecycle`, `language: "vhdl"` (`crates/cgg-core/src/frameworks/rules.rs:6767-6784`).
  - `detect: ["ieee", "IEEE", "std", "STD", "work", "WORK"]`, so it is detected by import prefix.
  - `attributes`, `registrars`, `base_types`, `methods`, `detect_paths`, `detect_calls` and `observer_types` are all `NONE`; `string_targets: false`; `node: true`.
  - It is **detect-only**: it enumerates no entries and carries a `gap:` string. Fixture a's stderr prints it under "seen, no rules: vhdl-toplevel — found in 2 file(s), entries NOT enumerated".
  - The gap string says the real entries cgg cannot see are:
    - the top-level entity that nothing instantiates;
    - testbench entities (no port list);
    - processes;
    - configuration and VHPI/foreign bindings.
  - Benchmark: `scripts/benchmark.sh:322` `"app-neorv32-vhdl|...|~vhdl-toplevel"`. The `~` means detected with a gap (benchmark.sh:120). Line 404 lists `vhdl-uvvm` with `~py-argparse` (UVVM's Python scripts).
- **No other rule names `vhdl`**: `grep -n '"vhdl"' rules.rs` hits only line 6769.
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`**: none apply.
- **Registrar verbs**: none, since `_ctx` is ignored.

## 11. Test harness entries

None. No `kind: test` rule exists for VHDL:

- VUnit (`library vunit_lib; context vunit_lib.vunit_context;`, `test_runner_setup`, `run("name")`) is not recognised;
- neither are UVVM or OSVVM testbenches, nor cocotb (a Python harness over VHDL).

Testbench entities (no ports) are not tagged as test roots. VHDL is not in `TEST_CLASS_MEMBERS_ARE_ROOTS`. There is no test-file gating. The dead-code capabilities row reports `test_tagging: none` (fixture a, `dc.json`).

## 12. Dead-code analysis

- **Capabilities row** (fixture a, `--dead-code --dead-code-format json`): `{"language":"vhdl","class":"degraded","visibility":"none","attributes":"full","value_references":"none","dispatch":"none","exports":"none","test_tagging":"none","max_confidence":"medium","root_rules_active":[]}`.
  - `attributes: full` is *observed*, not declared: the `entity`, `architecture`, `function` and `procedure` tags are non-empty `attributes`, so `caps.rs:133-135` counts them.
  - Visibility is never set, so the class is `Degraded` (`crates/cgg-resolve/src/deadcode/caps.rs:163-172`).
  - Only `Analyzable` with visibility and attributes can produce `High` (`signals_complete`, caps.rs:85-96), so **every VHDL finding caps at medium**.
- **Roots**: none.
  - VHDL has no `LANG_ENTRY` and no root rule (`grep -i vhdl` over `crates/cgg-resolve/src/deadcode/` and `crates/cgg/src/` is empty).
  - Fixture a: `roots 0 discovered · root-reachability disabled:no-roots-found`, which withholds `reachable-only-from-dead-code` (2 withheld).
- **Findings**: in fixture a all 9 non-entity callables are `never-referenced` at `medium`.
  - This includes `inc` and `log_msg`, which the process does call. The calls are invisible because call sites are not extracted (03), so for VHDL the dead-code report is essentially "everything except entities bound by an architecture".
  - Entities that an architecture's `of` reaches are referenced. `rtl` and `sim` (architectures) are reported as never-referenced (fixture a, `dc.json` findings).
- **No VHDL-specific blind spots**: `BLIND_SPOTS` (caps.rs:44) has no `vhdl` entry. VHDL is not in `SCRIPT_DRIVEN` or `DESCRIPTOR` (caps.rs:27, :41).
- **Not supported**: exported or public roots, the `implements-external-contract` cap, the `macro-generated` cap, the `reflection` cap, and unresolved-doubt handling. The last has nothing to work with, because there are no unresolved call sites: 0 in fixture a.

## 13. Builtins & stdlib tables

- **No `stdlib/vhdl.txt`**:
  - `ls crates/cgg-core/src/stdlib/ | grep -i vhd` is empty;
  - VHDL is absent from the language list in `crates/cgg-core/src/stdlib.rs:225-255`.
- `ieee.std_logic_1164`, `numeric_std`, `rising_edge`, `to_unsigned`, `resize` and the `std.textio` routines are not known as builtins. With no call sites they would not be consulted anyway.
- No `rust_types`-like tables. No `CAST_INTRINSICS` or `CONSTRUCTING_METHODS` entries (type conversions such as `unsigned(x)` / `integer(x)` are not modelled). No implicit packages: the implicit `library std, work; use std.standard.all;` is not modelled.
- `--include-stdlib` has no VHDL effect. `--include-external` would mint exit nodes for the `external_calls` bucket, for example `work.e1` or a case-mismatched entity name (fixture c, unverified with that flag).

## 14. Confidence & certainty

- **`high`**: a same-file architecture → entity reference, through `intra_file` (fixture a: both edges `high`, resolver `intra-file`).
- **`medium`**: a cross-file architecture → entity reference reached through a `use` unqualified prefix (fixture b: `beh2 -> Adder`, `medium`, `cross-file:imports`). In mermaid such an arrow carries the leading `?` label marker (CLAUDE.md, confidence section; fixture b was rendered as JSON only, so the `?` is unverified here).
- **Refused**:
  - cross-file without `use`: `candidates-in-other-files`;
  - two candidates: refused the same way;
  - selected name `work.e1` or a case mismatch: `external_calls`, `no-candidate-in-file`.
- **Not used**: no `CertainCtx` / `OwnerFacts` paths apply (typed receivers, imports confirming bases, arity narrowing). The `low` / `--guess-unique-names` path is Rust-only.

## 15. Dynamic use & reflection

Not applicable. VHDL is not covered by `dynuse` (CLAUDE.md lists Python, JS, Java, C#, Ruby and PHP). The language has no reflection beyond attributes (`'path_name`, `'instance_name`), and these are not read. Configuration-driven architecture selection is the closest analogue to dynamic dispatch, and it is not modelled (see 06).

## 16. Known gaps & limitations

- **No call-site extraction at all**: function calls, procedure calls (sequential and concurrent), component and entity instantiations, and configuration bindings. The module doc's "`function_call_expression` if present" (vhdl.rs:6-8) names a node kind the grammar does not have. The grammar's real kinds (`function_call`, `procedure_call_statement`, `concurrent_procedure_call_statement`, `component_instantiation_statement`) are unhandled. Consequences:
  - README UVVM benchmark: 1,036 callables, 0 edges (README.md:1350);
  - fixture a: 0 unresolved and 2 edges, both architecture→entity.
- **Instantiation hierarchy is invisible**. `entity work.counter` and `counter port map` produce no edge, so the design hierarchy, which is the main "call graph" of an HDL, is absent.
- **Case-sensitive name matching** in a case-insensitive language (fixture b: `of adder` does not bind `entity Adder`).
- **Processes are not extracted**. A procedure called only from a process reads as unreferenced. This is stated in the `vhdl-toplevel` gap (rules.rs:6780).
- **No package qualification**: package-body and protected-body subprograms get bare QNs. Same-named subprograms in different packages are indistinguishable by QN.
- **Operator overloads** (`function "+"`) and **subprogram declarations / prototypes** are not extracted (fixture a).
- **No library model** (`work.pkg` → file). There is no stdlib table, and `ieee` names are not classed as stdlib.
- **Cross-file entity binding needs a `use` clause**. Otherwise it is refused (fixture c). A selected `of work.e1` always goes external.
- **Dead `"include"` arm** for VHDL in cross_file.rs:2124-2129.
- **No test-harness or framework entries** (VUnit, UVVM, OSVVM, cocotb). `vhdl-toplevel` is detect-only.
- **Dead-code is capped at medium** (`degraded`: no visibility). With no call edges it would otherwise be mostly noise.
- **Tests**: only three inline unit tests (vhdl.rs:256-321), plus the `detect_prefixes.rs:90` import form. No ignored tests. No fixture-level tests under `crates/*/tests` cover VHDL call resolution. CHANGELOG.md mentions VHDL only in a perf-sampling note (line 2420); there are no VHDL feature entries.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | full | `.vhd` / `.vhdl` (case-insensitive ext); single grammar; generic depth/long-line skips only |
| 02. Callables extracted | partial | entities, architectures, subprogram bodies; no processes, packages, operators, prototypes, configurations; no package QNs |
| 03. Call-site forms | minimal | only architecture `of <entity>`; no function/procedure calls or instantiations |
| 04. Imports & module resolution | minimal | `library` / `use` recorded; `use` feeds unqualified prefixes; no library→file model; case-sensitive |
| 05. Type inference & receiver typing | none | no locals, types or receivers recorded |
| 06. Inheritance, dispatch & overloading | none | overloads extracted but not grouped; not in INHERITANCE / OVERLOAD lists |
| 07. Scoping & shadowing | minimal | architecture is the only scope; no locals or shadowing |
| 08. Macros, preprocessor & codegen | n/a | no macro system; generic instantiation not handled |
| 09. FFI & cross-language links | none | VHPI / foreign / mixed-HDL not linked |
| 10. Frameworks & entry points | minimal | `vhdl-toplevel` detect-only Lifecycle rule with gap string |
| 11. Test harness entries | none | no VUnit / UVVM / OSVVM / cocotb rules |
| 12. Dead-code analysis | minimal | class degraded, max medium, no roots; nearly every subprogram reported |
| 13. Builtins & stdlib tables | none | no `stdlib/vhdl.txt`; implicit std / work not modelled |
| 14. Confidence & certainty | minimal | intra-file arch→entity high; cross-file via `use` medium |
| 15. Dynamic use & reflection | n/a | not covered by dynuse; language has little reflection |
| 16. Known gaps & limitations | n/a | headline gap: zero call-site extraction (UVVM 1,036 callables / 0 edges) |
