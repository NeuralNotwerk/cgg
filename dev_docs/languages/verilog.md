# verilog

The Verilog / SystemVerilog plugin (`VerilogPlugin`, id `"verilog"`, `crates/cgg-lang/src/plugins/verilog.rs`, 301 lines including 3 unit tests) is one of the smallest plugins in cgg. It uses the crates.io grammar `tree-sitter-verilog` (`Cargo.toml:120` `tree-sitter-verilog = "1.0"`, locked at 1.0.3 in `Cargo.lock`; nothing vendored). It is registered at `crates/cgg-lang/src/plugins.rs:161`. The plugin has no `resolver_kind` override, no `signals()` override (so every `PluginSignals` bit is false, `crates/cgg-lang/src/lib.rs:304`), no `shebangs`, and no `max_line_bytes`. It extracts modules, tasks and functions as callables. It records two reference shapes: module/checker/program/udp/interface instantiation, and `$system_tf` calls. It records no imports, no locals, no types, no bases and no attributes beyond a kind tag. The plugin's own doc comment (`verilog.rs:1-8`) says that "The 'call graph' in Verilog comes mainly from `module_instantiation` … Plain task-enable statements … may not be captured". Resolution in practice is **intra-file only**, because no Verilog file ever has an import record, so the cross-file fallbacks never run (see §04).

## 01. Detection & parsing

- **Extensions** `.v`, `.vh`, `.sv`, `.svh` (`verilog.rs:22-24`). No other plugin claims these extensions: a grep of `crates/cgg-lang/src` for `".v"`/`".sv"`/`".vh"` finds only `verilog.rs`. So `.v` is never treated as Coq or V. Detection goes through `match_ext` (`crates/cgg-lang/src/detect.rs:134`), and the fixture audit shows `detected_via: "extension:.v"` / `"extension:.sv"`.
- **Shebangs**: none. The trait default `&[]` applies (`crates/cgg-lang/src/lib.rs:298`).
- **Content detection**: none. There is no header heuristic or sniffing for Verilog in `detect.rs`.
- **Grammar**: a single grammar, `tree_sitter_verilog::LANGUAGE` (`verilog.rs:25-27`), used for both Verilog-2005 and SystemVerilog. There is no dialect split by extension (`ts_language_for` keeps its default). `.sv` gets the same grammar as `.v`.
- **Re-parse / fallback**: none. A file with parse errors is still extracted from the error-recovered tree. Fixtures: `parity/verilog/a/top.sv`, `c/p2_class.sv`, `c/p5_import.sv` and `c/p6_dpi.sv` report `parse_status: "error"` but still yield partial callables. SV `class … extends`, `import pkg::*;` with `pkg::helper(2)` in an expression, and `import "DPI-C"` all produced parse errors in grammar 1.0.3 (fixtures under `scratchpad/parity/verilog/c`).
- **Skip conditions**: only the generic ones. These are `MAX_TREE_DEPTH` (4,000 levels, `SkipReason::TooDeep`, applied in `crates/cgg/src/lib.rs` to every language) and the walker's binary/deny-list classification. There is no per-language `max_line_bytes`. There is no minified-file handling, which is irrelevant here because `locals` are not recorded anyway.

## 02. Callables extracted

- **Module** `module counter(input clk); … endmodule`: becomes a `DefVariant::FreeFunction` named from the first `simple_identifier` under `module_header`, with `attributes: ["module"]` (`verilog.rs:88-111`). It pushes a scope, so nested definitions are qualified `counter.<name>`. Tests: `modules_are_callables` (`verilog.rs:261`), plus fixture `b/one.v` → `leaf`, `mid`, `top`.
- **Task** `task do_it; … endtask` and **function** `function [3:0] inc; … endfunction`: become an `InherentMethod` inside a module, or a `FreeFunction` at file scope. The name comes from the `task_identifier`/`function_identifier` descendant. The attribute is `"task"` or `"function"` (`verilog.rs:120-157`). Test: `task_and_function` (`verilog.rs:279`). Fixture `b/one.v` → `mid.twice`, `mid.show`.
- **Qualified names**: the enclosing module names joined with `.` (`qn`, `verilog.rs:58-64`). Only `module_declaration` pushes scope.
  - SV `package pkg; function int helper…; endpackage` gives a callable `helper`, not `pkg::helper` or `pkg.helper` (fixture `c/p1_pkg.sv`).
  - SV `class Base; virtual function void run(); … endclass` gives a `run:freefunction` with no owner. Two classes that both define `run` produce two indistinguishable `run` nodes (fixture `c/p2_class.sv`, `a/top.sv`).
- **Signature hint**: `super::extract_signature(text)` (`crates/cgg-lang/src/plugins.rs:74`). That routine looks for a `{` or a `:` body delimiter, so for Verilog it usually returns the whole collapsed definition text (dead-code JSON `signature_hint: "module top; wire a, y; mid m0(.a(a), .y(y)); initial $finish; endmodule"`).
- **Not extracted**, verified on fixtures:
  - `interface … endinterface`, `program … endprogram` and `package` (as a node), `class` (as a node or an owner), `checker`, `primitive` (UDP), `config`, and `always`/`initial` blocks (fixture `c/p3_iface.sv` → no callables).
  - SV constructor `function new(); … endfunction` inside a class: absent from fixture `c/p2_class.sv` and `a/top.sv`. The grammar uses a separate constructor node that the walker does not match.
  - `import "DPI-C" function int c_add(...)` prototypes: absent (fixture `c/p6_dpi.sv`).
  - `extern`/`pure virtual` prototypes: no handling in the code (unverified on fixture).
  - `generate` blocks (named or not) are not scopes. Instances inside them belong to the enclosing module (fixture `c/p4_gen.v`).
- **Overloads/clauses, operators, closures, properties**: not applicable to the language, or not modelled.
- **Abstract/interface members**: SV `pure virtual` / `interface class` members are not tagged `abstract`. There is no code for them.
- **Generated code**: none. See §08.

## 03. Call-site forms

- **Module instantiation** `counter c0(.clk(clk), .q(q));`, positional `leaf l1(a, y);`, and parameterised `counter #(.W(4)) c1(...)`: each records a `RefRecord { name: <module type>, receiver_hint: "" }`. The name is the first `simple_identifier` under the node (`verilog.rs:165-183`). The same handling covers `checker_instantiation` (which the grammar emits for an ambiguous `foo bar();`), `program_instantiation`, `udp_instantiation` and `interface_instantiation`. Fixture `b/one.v`: `mid -->|2x| leaf`, `top --> mid`, both `high` / `intra-file`. Fixture `a/top.sv` line 19 shows the parameterised form recorded as `counter`.
- **Interface instance** `bus_if b();`: recorded as a reference. The interface declaration itself is not a callable, so the reference goes to the external bucket (`a/top.sv` audit: `external_calls 20 bus_if`).
- **System task/function** `$display("x")`, `$finish`: recorded as a `RefRecord` named with the leading `$` (`verilog.rs:184-201`, test `system_call_captured`). No callable ever carries the name, so these always go to the external bucket (`name_screen_applied: "external"`). With `--include-external` they become `<external>::$display` leaf nodes (fixture `b`).
- **Not recorded**:
  - **Task enable** `show;` / `show();` / `do_it(1);`: no `RefRecord`. Fixture `b/one.v` has no edge `mid → mid.show`, and the plugin header comment at `verilog.rs:6-8` says so too.
  - **Function call in an expression** `twice(3)`, `inc(q)`: no `RefRecord` (fixture `b`, `a`).
  - **Package-scoped call** `pkg::helper(2)` and class method calls `obj.run()`, `super.new()`, `super.run()`: not recorded (fixtures `c/p5_import.sv`, `c/p2_class.sv`).
  - **Hierarchical references** `top.m0.sig` and **macro invocations** `` `INST(x0) ``: not recorded (fixture `c/p4_gen.v` → only the literal `leaf u();` edge).
  - **DPI calls** `c_add(1,2)`: not recorded.
- **Arity, generics, value refs, registrar arguments, paren-less flags, receivers**: none are set (`RefRecord` is built with `..Default::default()` and only `name`, `site_line`, `site_byte`).

## 04. Imports & module resolution

- **Imports recorded**: none. `verilog.rs` never pushes to `facts.imports`.
- **`` `include "file.v" ``**: not recorded. `crates/cgg-resolve/src/cross_file.rs:2123-2140` has an `"include"` arm that lists `lang == "verilog"` and tries `{path}.v`. That arm is unreachable, because the plugin emits no `ImportRecord`. Fixture `a/top.sv` (`` `include "counter.v" `` plus `counter c0(...)`) still ends `candidates-in-other-files {candidates: 1}`.
- **Doc claims disagree with the code**: the `crates/cgg/tests/detect_prefixes.rs:91` import form for verilog is `` `include "<prefix>.v" ``, and CHANGELOG.md:5471-5475 says Verilog "parses `` `include ``". The plugin does not record includes. README.md:522 is accurate ("`` `include `` yields no edges").
- **SV `import pkg::*;` / `import pkg::helper;`**: not recorded. They also cause parse errors in grammar 1.0.3 (fixture `c/p5_import.sv`).
- **Cross-file resolution**: none in practice.
  - Step 1d (global by-simple fallback, `cross_file.rs:6015-6022`) requires `has_imports`, and Verilog files never have any. A module instantiated in one file and defined in another is refused with `no-candidate-in-file` followed by `candidates-in-other-files` (fixture `a`).
  - README benchmark row: picorv32 has 0% cross-file (`README.md:1349`).
- **FileFacts::packages, manifests, heads.rs external refusal, modules.rs, libscope.rs, not-in-imported-module**: none apply. `"verilog"` appears nowhere in those files (grep).

## 05. Type inference & receiver typing

Not applicable: the plugin records no parameters, locals, types or receivers. `"verilog"` does not appear in `type_hints.rs`, `closure_types.rs`, `receiver.rs` or `declared_type` (grep over `crates/cgg-resolve/src` and `crates/cgg-lang/src`). SV class handles (`Base b = new;`, `b.run()`) are neither typed nor recorded.

## 06. Inheritance, dispatch & overloading

- **Base records**: none. SV `class Derived extends Base` records no `ClassDecl` or base, and classes are not even owners (fixture `c/p2_class.sv`).
- **super calls**: `super.new()` and `super.run()` are not recorded.
- **Virtual dispatch / interface classes**: not modelled. `"verilog"` is not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS`, and is not anywhere in `dispatch.rs` (grep). `--dynamic-dispatch` changes nothing: fixture `b` gives the identical graph.
- **member_of_calling_class, overrides_below, extension methods, mixins, arity overload narrowing**: none.
- The dead-code capability table reports `dispatch: none` (fixture `d`).

## 07. Scoping & shadowing

- **FileFacts::locals**: not recorded. `"verilog"` does not appear in `crates/cgg-lang/src/locals.rs`. There is no `shadowed-by-local`.
- **Lexical narrowing**: Verilog is not in the lexical-narrowing language list (CLAUDE.md architecture §3, and grep of `intra_file.rs` finds no `verilog`). Intra-file linking matches by simple name across the whole file.
- **Same-name definitions in one file**: two `module leaf` under `` `ifdef FAST … `else … `endif `` make the instantiation `leaf l();` refused as `ambiguous-in-file`, so no edge is drawn (fixture `e/dup.v`). Verilog has no "preprocessor alternatives bind once" rule like C's.
- **Conditional definitions**: not tagged (`DefRecord::conditional` unset).

## 08. Macros, preprocessor & codegen

- **`` `define `` / macro use `` `INST(x0) ``**: not expanded, not recorded, and no `name_mentions` (fixture `c/p4_gen.v`). `c_macro.rs` handles only C/C++.
- **`` `ifdef``/`` `else``/`` `endif ``**: the grammar sees both branches. Both definitions are extracted, which causes the ambiguity in §07.
- **`generate for/if`**: the instances inside are recorded (fixture `c/p4_gen.v` gives the `g → leaf` edge). The generate block is not a scope.
- **Parameters / `defparam`**: not modelled. A parameterised instantiation still records the module name.
- **macro:<name> tagging, name_mentions**: none.

## 09. FFI & cross-language links

- **SystemVerilog DPI-C** (`import "DPI-C" function …`, `export "DPI-C" …`): not handled. A grep for `DPI` in `crates/cgg-resolve/src/ffi.rs` finds nothing, and fixture `c/p6_dpi.sv` produces no FFI edge.
- **PyO3/wasm-bindgen/napi/JNI/P/Invoke/extern C/asm**: not applicable.
- **Descriptor links, markup code-behind**: not applicable.

## 10. Frameworks & entry points

- **Rules applying to verilog**: none. `crates/cgg-core/src/frameworks/rules.rs` contains no `verilog` rule (grep). Two tests depend on that:
  - `crates/cgg/tests/frameworks.rs::coverage_discloses_languages_with_no_rules_at_all` (line ~440) uses a `.v` file as the "no framework rules" exemplar.
  - `crates/cgg-resolve/src/frameworks/mod.rs::languages_with_no_rules_at_all_are_disclosed` (line ~2541) does the same.
- **Framework coverage**: fixture `a` audit gives `no_markers: [{language: "verilog", files: 2}]`, and `scripts/benchmark.sh:403` has an empty APPS list for `verilog-picorv32`.
- **member_sources, signature_methods, container_attributes, markup_kinds, registrar verbs**: none.
- **No testbench or simulator conventions** are modelled (UVM `uvm_test`/`build_phase`, cocotb, top-level uninstantiated modules).

## 11. Test harness entries

- **No `kind: test` rules** for verilog (no rules at all, §10). There are no UVM, SVUnit or cocotb harness nodes.
- **Built-in name rule only**: the dead-code pass's default arm of `is_test_name` (`crates/cgg-resolve/src/deadcode/roots.rs:975-988`) is `name.starts_with("test_") || name.starts_with("Test")`. Modules named `test_top` and `TestBench` become `test-entry` roots via `builtin:test-name`. The common `tb_top` / `*_tb` naming does not (fixture `d/tb.v`).
- **TEST_CLASS_MEMBERS_ARE_ROOTS, test-file gating**: not applicable.

## 12. Dead-code analysis

- **Capability row** (fixture `d`): `class: degraded`, `visibility: none`, `attributes: full` (because every def carries a kind attribute), `value_references: none`, `dispatch: none`, `exports: none`, `test_tagging: none`, `max_confidence: medium`. The class logic is at `crates/cgg-resolve/src/deadcode/caps.rs:163-172`.
- **Roots that fire**:
  - **Bug (verified)**: every Verilog **task** is rooted as `framework-callback` by `builtin:framework-attr` with detail `task`. The plugin tags tasks with the attribute `"task"` (`verilog.rs:151-152`), and that string is in `FRAMEWORK_ATTRS` (`crates/cgg-resolve/src/deadcode/roots.rs:199`, meant for Celery's `@task`). Fixtures `b` (`mid.show`) and `d` (`m.t`) show this. Functions (`"function"`) and modules (`"module"`) are not affected, so in fixture `d` `m.t` is a root and `m.f` is reported.
  - `builtin:test-name` for `test_*`/`Test*` names (§11).
  - `main` (`UNIVERSAL_ENTRY`, `roots.rs:20`) applies by name. This was not tested on a fixture.
  - No `LANG_ENTRY` entry for verilog (`roots.rs:23-31`).
  - The top-level uninstantiated module (the real design or testbench root) is **not** a root. It is reported `never-referenced` (fixture `b`: `top`, fixture `d`: `tb_top`).
- **Evidence on findings**: `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges`, `no-unresolved-site-with-name`. Confidence is capped at medium.
- **Missed callers**: because task/function calls are never recorded, every function and every non-rooted task is reported dead even when it is called (fixture `b`: `mid.twice` is used by `show` but reported).
- **Withholding**: root coverage below 25% withholds findings (fixture `b`: "2 withheld (root coverage too low)").
- **implements-external-contract, macro-generated, reflection caps**: not applicable.

## 13. Builtins & stdlib tables

- **`stdlib/verilog.txt`**: absent. `crates/cgg-core/src/stdlib/` has no verilog file, and `stdlib_names` (`crates/cgg-core/src/stdlib.rs:221-258`) has no `"verilog"` entry.
- **System tasks** `$display`/`$finish`/…: classified **external**, not stdlib. `--include-stdlib` adds nothing. `--include-external` mints `<external>::$display` nodes (fixture `b`).
- **CAST_INTRINSICS, CONSTRUCTING_METHODS, rust_types-like tables, implicit packages**: none.

## 14. Confidence & certainty

- **Intra-file module instantiation** to a uniquely named module in the same file: `high`, resolver `intra-file` (fixture `b` edges). This is the only edge-producing path.
- **Ambiguous same-file name**: refused (`ambiguous-in-file`), never `medium` (fixture `e`).
- **Cross-file**: never resolved, so there are no `medium`/`low` edges in practice.
  - `--guess-unique-names` is Rust-only (CLAUDE.md), so it does not apply.
  - The mermaid `?` marker never appears for Verilog edges (all edges are `high`).

## 15. Dynamic use & reflection

Not applicable / none. The dynuse covered languages are Python, JS, Java, C#, Ruby and PHP (CLAUDE.md §7). The plugin sets no `dyn_uses` signal. String-based instance selection (`$cast`, a `uvm_factory` `create("name")`, `+UVM_TESTNAME`) is not modelled.

## 16. Known gaps & limitations

- Task-enable statements and function calls in expressions are not recorded (`verilog.rs:6-8`, fixture `b`). This is the main gap. It is also why cross-file resolution and dead-code accuracy are near zero. README.md:522 and `skills/cgg/SKILL.md:501-502` document it.
- No import/include records, so there are no cross-file edges even when module instantiation crosses files (fixture `a`, README benchmark picorv32 0%). The `cross_file.rs:2127` `verilog` include arm is dead code, and CHANGELOG.md:5472's "it parses `` `include ``" is inaccurate.
- The task attribute `"task"` collides with `FRAMEWORK_ATTRS` `task` (`roots.rs:199`), so every task is wrongly a dead-code root (fixtures `b`, `d`).
- SystemVerilog OOP is not modelled: classes are not owners, `new` is not extracted, and there are no `extends` bases, no method calls, no `super`. Packages are not scopes (`pkg::f` qualified as `f`). Interfaces and programs are not callables. Several SV constructs produce parse errors with `tree-sitter-verilog` 1.0.3 (fixtures `c/p2`, `c/p5`, `c/p6`).
- `` `ifdef `` alternatives make an instantiation ambiguous and refused (fixture `e`). Macros are not expanded (fixture `c/p4_gen.v`).
- DPI-C is not linked. There is no stdlib table. Uninstantiated top modules and testbenches are not roots. There are no UVM/cocotb framework rules.
- No `#[ignore]` tests concern verilog (only 3 unit tests in `verilog.rs`, none ignored).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.v/.vh/.sv/.svh` by extension, single grammar; many SV constructs parse with errors, no fallback |
| 02 Callables extracted | partial | modules, tasks, functions only; no class/package/interface/program/`new` |
| 03 Call-site forms | minimal | module/interface instantiation and `$system` calls only; task/function calls not recorded |
| 04 Imports & module resolution | none | no import or `` `include `` records; cross-file never binds |
| 05 Type inference | n/a | nothing typed |
| 06 Inheritance/dispatch | none | SV `extends`/virtual/super unmodelled |
| 07 Scoping & shadowing | minimal | module scope for qualified names only; no locals; `ifdef` twins ambiguous |
| 08 Macros/preprocessor | none | no `` `define `` expansion, no name_mentions |
| 09 FFI | none | DPI-C not linked |
| 10 Frameworks | none | no rules; verilog is the test exemplar for "no framework rules" |
| 11 Test harness | minimal | only generic `test_*`/`Test*` name roots; no UVM/cocotb |
| 12 Dead-code | minimal | degraded, medium cap; tasks falsely rooted via `task` attr; called functions reported dead |
| 13 Builtins | none | no stdlib table; `$display` etc. counted external |
| 14 Confidence | partial | intra-file instantiation is `high`; nothing else binds |
| 15 Dynamic use | n/a | not covered |
| 16 Known gaps | n/a | see list: calls, includes, SV OOP, attr collision |
