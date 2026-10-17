# ocaml

The OCaml plugin (`crates/cgg-lang/src/plugins/ocaml.rs`, 360 lines, `OcamlPlugin`, id `"ocaml"`) is a small hand-written tree-sitter walker over the `tree-sitter-ocaml` crate (`Cargo.toml:113`, `tree-sitter-ocaml = "0.25"`, Cargo.lock 0.25.0), bound as `tree_sitter_ocaml::LANGUAGE_OCAML` (ocaml.rs:21-23). Nothing is vendored. It does five things: it turns every `let_binding` / `value_definition` that has a `value_name` into a `DefRecord { variant: FreeFunction }` whose qualified name is the bare name (ocaml.rs:202-237); it records every `application_expression` whose head is a `value_name` / `value_path` as a `RefRecord` with the full written path as `name` and an empty `receiver_hint` (ocaml.rs:239-268); it records `open` as an `ImportRecord { kind: "open" }` (ocaml.rs:181-200); it records the top module of every qualified path once per file as `ImportRecord { kind: "module-use" }`, which only framework detection reads (ocaml.rs:72-84, 139-179); and it sets `RefRecord::not_def` for calls to the enclosing non-`rec` `let` (ocaml.rs:248-257). There is no `resolver_kind` method any more (grep finds none in `crates/`). `ExtractCtx` is ignored (`_ctx`, ocaml.rs:27). The plugin does not override `signals()` (default `PluginSignals::default()`, `crates/cgg-lang/src/lib.rs:304`), `max_line_bytes()` or `ts_language_for()`, so it declares no visibility, attributes, exports, test defs, value refs or dynamic uses. OCaml-specific resolver code: lexical scoping and positional shadowing in `intra_file.rs` (:119-136, :400-415), the `open` import table and the `open A.B` → `a.ml` file rule in `cross_file.rs` (:1916-1925, :2064, :2178-2206), and `not_def` filtering (`cross_file.rs:2596`, `intra_file.rs:264`); locals come from `OCAML_SPEC` in `crates/cgg-lang/src/locals.rs:344-366`.

Fixtures used for this document live in `<scratch>/parity/ocaml/` (`f1/` general shapes: modules, classes, externals, operators, opens, `.mli`; `f2/` parameters, shadowing, dead code; `f3/` Dream / Alcotest / Cmdliner / Opium; `f4/` bare cross-file names, stdlib exit nodes; `f5/` `include`, local opens, module aliases, functors, ppx, `let rec … and`; `f6/` minimal repro of a wrong high edge; `f7/` top-level invocation roots), run with cgg 0.11.0 (`target/release/cgg`). `show.py` there dumps nodes, edges, unresolved sites and files from `-t json` output.

## 01. Detection & parsing

- **Extensions** `.ml` and `.mli` (ocaml.rs:15-17; unit test `plugin_loads`, ocaml.rs:293). Fixture f1: `a.ml` `detected_via: extension:.ml`, `b.mli` `detected_via: extension:.mli`.
- **Shebangs**: none (`&[]`, ocaml.rs:18-20). `#!/usr/bin/env ocaml` scripts without an extension are not detected (unverified by fixture; follows from the empty list and no content rule).
- **Content detection**: none (no OCaml entry in `crates/cgg-lang/src/detect.rs`; grep for `ocaml`/`.ml` finds nothing).
- **Grammar**: one grammar, `LANGUAGE_OCAML`, for both `.ml` and `.mli` (ocaml.rs:21-23). The crate's separate interface grammar (`LANGUAGE_OCAML_INTERFACE`) is not used; `.mli` files still parse (`b.mli` with `val foo : int -> int` and a `module Sub : sig … end` → `parse_status ok`, fixture f1) but yield no callables (see 02).
- **Not detected**: `.mll` (ocamllex), `.mly` (menhir), `.eliom`, `.re`/`.rei` (Reason), `dune`/`dune-project`/`.opam` files — no extension registered (ocaml.rs:16).
- **Re-parse / fallback**: none. A file with syntax errors is extracted from tree-sitter's error-recovered tree (generic path, `crates/cgg/src/lib.rs:451-458`).
- **Depth skip**: generic — a tree deeper than `MAX_TREE_DEPTH` (4,000, `crates/cgg/src/lib.rs:155`) is skipped as `SkipReason::TooDeep` (lib.rs:451-454). Not exercised with an OCaml fixture.
- **Minified files**: generic — a line over `MINIFIED_LINE_BYTES` (2,000, lib.rs:159) disables `locals::collect`; extraction still runs.
- **Long-line skip**: `max_line_bytes()` not overridden → no cap.

## 02. Callables extracted

Every callable is `DefVariant::FreeFunction` (`kind: function` in output), `simple_name == qualified_name ==` the first `value_name`/`identifier` child of the `let_binding`/`value_definition` (ocaml.rs:202-237), `signature_hint = extract_signature(text)` (`plugins.rs:74`), `visibility: ""`, `attributes: []`.

Supported:

- **Top-level function** `let helper x = x + 1` → `helper`. Unit test `let_bindings_and_call` (ocaml.rs:301); fixture f1.
- **`let rec`** `let rec loop n = … loop (n - 1)` → `loop`, self-edge high intra-file (fixture f1).
- **Mutual recursion** `let rec even n = … odd … and odd n = … even …` → `even`, `odd`, edges both ways high (fixture f5 `misc.ml`).
- **Nested `let … in`** `let main () = let inner y = helper y in …` → `inner` is its own callable (fixture f1: `inner` at line 8, `main → … inner`). Lexical narrowing makes a bare call bind the innermost one (see 07).
- **Anonymous function bound by name** `let anon = fun x -> helper x` → callable `anon` with edge `anon → helper` (fixture f1).
- **Functions inside `module M = struct … end`** → extracted, but with **bare** qualified names: `mfun`, `nfun` (fixture f1 `M.mfun`, `M.N.nfun` both have qn `mfun`/`nfun`). `extract_module` (ocaml.rs:127-137) looks for a `module_name` *direct child* of `module_definition`, but the grammar nests it under `module_binding`, so `self.module` is never set and `qn()` (ocaml.rs:58-64) always returns the simple name. The `M::name` formatting branch is dead in practice.
- **Functor bodies** `module F (A : …) = struct let get () = A.v end` → `get` (bare qn; fixture f5).
- **ppx-extended let** `let%lwt fetched = fetch ()` → callable `fetched` (fixture f5).
- **Labeled / optional parameters** `let labeled ~f x = f x`, `let opt ?(g = f) x = g x` → `labeled`, `opt` (fixtures f1, f2).

Over-extraction (shapes turned into callables that are not functions):

- **Plain values** `let value = 42`, `let w = Obj.magic 1`, `let pp = [%expr 1 + 1]` → callables `value`, `w`, `pp` (fixtures f1, f5). Every `let` with a `value_name` pattern is a node, function or not.
- **Wildcard bindings** `let _ = inner 3 in …` → a callable literally named `_`, one per binding (fixture f1: six `_` nodes inside `main`; f3 `op.ml` top-level `let _ = … |> Opium.App.run_command`). Calls in those bodies are attributed to the `_` node, not to the enclosing function (f1: `B.foo` site's `src` is a `_` node).
- **Local value bindings** `let f = g in f 2`, `let c = new counter in c#incr` → callables `f`, `c` (fixtures f2, f1). This causes wrong edges (see 07).

Not extracted:

- **`let () = …`** (unit pattern) — no callable; calls inside are `no-enclosing-callable` (fixture f1 line 16, f7). Documented in the cmdliner/dream gaps (rules.rs:6042, 6093).
- **Tuple / record / constructor patterns as the bound name** `let x, y = 1, 2` yields nothing (fixture f5); a tuple *parameter* is fine (`let tup (a, b) = …` → `tup`).
- **Operator definitions** `let ( +! ) a b = a + b` — the name is a `parenthesized_operator`, not `value_name`; not extracted (fixture f1).
- **Classes and objects** `class counter = object method incr = … method get = … end` — no `class_definition`/`method_definition` handling; neither the class nor its methods are callables (fixture f1).
- **`external` declarations** `external c_add : int -> int -> int = "caml_add"` — not extracted (fixture f1; `call_ext → c_add` goes to the external bucket).
- **`.mli` signatures** `val foo : int -> int`, `module type S = sig val run : … end` — `value_specification` is not walked; no callables, no interface/abstract members (fixture f1 `b.mli`, f5).
- **Modules / functors themselves** — no node for a module, functor or module type.
- **Variant constructors, record fields, type definitions** — not callables.
- **Owner / container information** — none: no `type_spans`, no `ClassDecl`, no `owner`.

## 03. Call-site forms

Recorded only from `application_expression` whose first child is `value_name`, `value_path` or `identifier` (ocaml.rs:107-110, 239-268). `name` is the head's full text; `receiver_hint` is always empty; `arity`, `path_receiver`, `value_ref`, `typed_receiver`, `paren_less` are never set.

Supported:

- **Bare application** `helper 41`, `loop (n - 1)`, `f ()` → `RefRecord { name: "helper" }` (unit test `let_bindings_and_call`; fixture f1).
- **Qualified application** `List.map helper [1;2]`, `B.foo 1`, `B.Sub.deep 2` → `name: "List.map"` / `"B.foo"` / `"B.Sub.deep"` (dotted text, no receiver split). Recorded, but see 04: a workspace `B.foo` never resolves (fixture f1: `B.foo`, `B.Sub.deep`, `M.mfun`, `L.length` all `external` / `no-candidate-in-file`).
- **Application with labeled/optional args** `labeled ~f:helper 3` → `labeled` (edge high, fixture f1).
- **Local-open application** `B.(foo 5)`, `Alcotest.(check int) …` → the inner call is recorded as bare `foo` / `check` (fixtures f1, f3); `B.( … )` itself records no import (see 04).
- **Self-call in non-`rec` let** carries `not_def` (see 07).

Not recorded:

- **Pipelines and application operators** `3 |> helper`, `helper @@ 4`, `Dream.run @@ Dream.logger @@ …`, `x |> Opium.App.get "/" h` — these are `infix_expression`s, not `application_expression`s; the function on the operator side is not recorded (fixture f1 lines 10-11: no edge to `helper`; f3: no `Dream.run`, no `Opium.App.run_command`). Arguments that are themselves applications (`Dream.router [ … ]`, `Opium.App.get "/" h`) are still recorded.
- **User-defined infix operators** `1 +! 2` — not recorded (fixture f1 `use_op` has no edge).
- **Method calls** `c#incr`, `self#get` (`method_invocation`) — not recorded (fixture f1).
- **Object construction** `new counter` — not recorded.
- **Functor application** `Map.Make(String)` — not a call (fixture f1 `functor_use` has no out-edge).
- **Functions passed as values** `List.map helper l`, `app f 1`, `labeled ~f:helper`, `Alcotest.test_case "add" \`Quick test_add`,`Term.const run` — no value references, no registrar capture; `--reference-edges` adds nothing (fixture f1: 14 edges with and without the flag). Explicitly named as the blocker in all four framework rule gaps (rules.rs:6042, 6059, 6076, 6093).
- **First-class module calls** `M.run ()` with `(module M : S)` — recorded as `M.run`, never resolved (fixture f5).
- **Constructors / variants** `Some x` — not calls (by design).
- **Arity** — never recorded; OCaml is not in `OVERLOAD_ARITY_LANGS` (`type_hints.rs:1583`) or `is_clause_language` (`intra_file.rs:858-860`).

## 04. Imports & module resolution

Supported:

- **`open M`** / **`open A.B`** → `ImportRecord { kind: "open", path: "M" }` (ocaml.rs:85-88, 181-200; unit test `open_is_recorded`, ocaml.rs:318). In `cross_file.rs:1916-1925` an `open` pushes the path onto `unqualified_prefixes` and aliases its last segment; `cross_file.rs:2064` also routes the generic dotted-module branch for OCaml. A bare call then resolves through step 1c (scoped by-simple lookup over files whose path matches the prefix, capped at 8 candidates, `cross_file.rs:5990-6000`) → edge `cross-file:imports`, **medium** (fixture f1 `use_open → foo@b.ml`).
- **`open A.B` reaches file `a.ml`** — the head of a dotted open names the compilation unit (`cross_file.rs:2178-2186`, `2200-2206`). Test `an_ocaml_open_of_a_submodule_reaches_its_file` (cross_file.rs:14593); CHANGELOG.md:231.
- **`let open B in …`** (`open_module` node) → `open` import (ocaml.rs:85). Fixture f5 `lo.ml`: `a → foo@b.ml` medium. Note the import is file-wide, not scoped to the `in` body.
- **Copies narrowing** — same-named definitions in several files with no evidence for one bind nothing (`narrow_copies`, cross_file.rs:3794; test `copies_of_a_name_need_evidence` second half, cross_file.rs:12721-12745: two same-directory `k`).
- **Bare name with no `open`** → not bound across files (`candidates-in-other-files`), even when globally unique (fixture f4 `bar → foo`, 1 candidate). This matches OCaml semantics.
- **`module-use` records** — top module of every `value_path`/`constructor_path`/`field_path`/`type_constructor_path`/`module_path`, once per file (ocaml.rs:72-84, 139-179; test `qualified_names_record_their_top_module_once`, ocaml.rs:325). Used only for framework detection; "the resolver ignores the kind" (ocaml.rs:75-76).

Not supported / partial:

- **Qualified calls into workspace modules** `B.foo 1` (no `open`) — never resolved. The site name is `"B.foo"`; no callable has that qn (module qualification is never formed, see 02) and step 1b/1c only try `<prefix>.<name>` / bare names, so the site lands in the **external** bucket (`no-candidate-in-file`, `name_screen_applied: external`, `crates/cgg-core/src/external.rs:159-163`). Fixture f1: `B.foo`, `B.Sub.deep`, `M.mfun` (same file!). This is the dominant OCaml call form.
- **`include M`** — not recorded (no `include_module` handling in ocaml.rs). Fixture f5 `inc.ml` (`include B` + `foo 1`) → `candidates-in-other-files`. README.md:515 claims "open, include"; only `open` is implemented.
- **Local open `M.(expr)`** — records no import (fixture f5 `dot.ml` → `candidates-in-other-files`); in f1 it resolved only because the file also had `open B`.
- **Module aliases** `module X = B` then `X.foo` — not recorded; no edge (fixture f5 `alias.ml`). CHANGELOG.md:1349 lists "OCaml `open`/local-open/module aliases" as not covered (WS14); `open A.B` was later fixed, local-open and aliases still are not.
- **Functors / first-class modules** — no module-level resolution.
- **`FileFacts::packages`** — not set; no dune/opam manifest reading (`manifest.rs` has no OCaml reader).
- **External-module refusal (`heads.rs`)** — OCaml is not a `WorkspaceVocab`/`FileHeads` language (Python/JS/TS/Go/Java/Kotlin/C#/C++ only); no `external-module` audit reason is produced for OCaml.
- **`not-in-imported-module` (`modules.rs`)** — not applicable; OCaml has no `ModuleIndex` entry.
- **`.mli` ↔ `.ml` pairing** — none; interface files contribute imports only.

## 05. Type inference & receiver typing

Not applicable in practice: the plugin never sets `receiver_hint`, `local_types`, `typed_receiver`, `weak_receiver_type`, `implicit_receiver`, `scoped_types` or `arg_type` (ocaml.rs:258-265 sets only `name`, `site_line`, `site_byte`, `not_def`). `type_hints.rs`, `closure_types.rs` and `declared_type` have no OCaml branch (grep finds none). OCaml calls are module-qualified rather than receiver-based, so no typed-owner lookup ever runs; object method calls (`obj#m`) are not recorded at all (see 03).

- Parameter / local / field / return-type typing: none.
- Casts / constructions (`CAST_INTRINSICS`, `CONSTRUCTING_METHODS`): no OCaml entries used.
- Wrappers / nullability (`option`): not read.
- Closure parameter typing: none.

## 06. Inheritance, dispatch & overloading

None.

- **Base-class records / `inherit`** — OCaml classes are not extracted (02), so `class c = object inherit base … end` records nothing.
- **`super`, `member_of_calling_class`, `overrides_below`, `resolve_via_bases`** — not reached (no owners).
- **Module signatures / functors as interfaces** — no `trait_impl_target`; `--dynamic-dispatch` adds nothing.
- **`inheritance_fanout`** — OCaml is not in `INHERITANCE_LANGS` (`crates/cgg-resolve/src/dispatch.rs:110-123`).
- **Overload / arity narrowing** — not in `OVERLOAD_ARITY_LANGS` (`type_hints.rs:1583`); OCaml has no overloading, so this is n/a. Same-name redefinitions are handled by positional shadowing instead (07).
- **Polymorphic variants / GADTs / first-class modules** — not modeled.

## 07. Scoping & shadowing

Supported:

- **`FileFacts::locals`** — `OCAML_SPEC` (`locals.rs:344-366`): `value_pattern` identifiers bound by `parameter` (param scope) and `match_case` patterns (own scope), with function scopes `let_binding`, `fun_expression`, `function_expression`, and the destructuring pattern kinds listed. Test `dart_ocaml_lua_python_rust_kotlin_swift_scala_zig` (locals.rs:840, OCaml case at :858: `let iter f s = List.iter (fun e -> f e) s` → locals `f`, `s`, `e`).
- **`shadowed-by-local`** — a bare call through a parameter is not a function: `let app g x = g x`, `let lab ~g x = g x`, `let opt ?(g = f) x = g x`, `fun f -> f 1`, `match x with Some f -> f 1` → `shadowed-by-local` (fixtures f2, f6 `a.ml`). CHANGELOG.md:566.
- **Lexical narrowing of nested definitions** — OCaml is in `lexically_scoped` (`intra_file.rs:119-136`): a call binds the innermost visible nested `let`. Test `nested_functions_are_scoped_lexically_in_ml_style_languages` (intra_file.rs:1187; three functions each with `let rec go`). CHANGELOG.md:578-582.
- **Positional shadowing** — a later same-scope binding shadows an earlier one: the candidate is the last definition starting before the site (`intra_file.rs:400-415`). Fixture f2: `after () = f 9` → the second top-level `f` (line 12); `test_thing` (line 10) → the first `f` (line 2).
- **Non-recursive `let` does not see itself** — `let helper x = helper (x * 2)` sets `not_def` to the binding's start byte (ocaml.rs:248-257, walker stack `lets`, ocaml.rs:89-102); intra-file and cross-file drop that candidate (`intra_file.rs:264`, `cross_file.rs:2596`). Under `let rec` it is itself. Tests `a_non_recursive_let_does_not_see_itself` (ocaml.rs:341), `a_non_recursive_binding_is_not_its_own_callee` (intra_file.rs:1136). CHANGELOG.md:703-704.

Partial / wrong:

- **A parameter does not shadow another function's local `let`** — because a local `let f = …` inside some other function is a callable (02), and the locals rule keeps nested definitions as candidates (the "local may hold a closure" rule, intra_file.rs test `a_local_may_hold_a_closure_a_factory_returns`), `let app2 = fun f -> f 1` binds to `let f = 3 in f` *inside a different function* at **high** (fixture f6 `b.ml`: `app2 → f@b.ml:3`; with no such nested `f`, `a.ml` correctly reports `shadowed-by-local`). Same in f2 (`app2`, `m` → `f@8`, the `let f = g` inside `shadow`). Root cause beyond this observation unverified.
- **`not_def` with patterns** — the `lets` stack keys on the full pattern text (ocaml.rs:95-98), so it only matches simple names.
- **Conditional definitions** — n/a (no `DefRecord::conditional`).
- **`let open … in` scope** — treated as a file-wide `open` (04).

## 08. Macros, preprocessor & codegen

Not applicable / none:

- **ppx** (`[%expr …]`, `let%lwt`, `[@@deriving show]`) — not expanded. `let%lwt x = e` is extracted as a normal binding (`fetched`, fixture f5); `[@@deriving …]` generates no callables (`show_t`/`pp_t` are invisible) and attributes are not recorded (`attributes: Vec::new()`, ocaml.rs:234).
- **cppo / `#if`** — not handled.
- **`macro:<name>` tagging** — never set.
- **`FileFacts::name_mentions`** — never set.
- **ocamllex / menhir generated code** — `.mll`/`.mly` not detected (01); their generated `.ml` is analysed only if checked in.

## 09. FFI & cross-language links

None.

- **`external` stubs** `external c_add : … = "caml_add"` — not extracted (02), and `ffi::link_ffi` has no OCaml family (grep `ocaml` in `crates/cgg-resolve/src/ffi.rs` finds nothing). So OCaml→C stubs (`caml_*` functions in C) and C→OCaml `Callback.register` are not linked. Fixture f1: `c_add` call → external bucket.
- **js_of_ocaml / Melange / ctypes** — not handled.
- **Descriptor links (protobuf rpc → host)** — no OCaml host binding in `descriptor.rs`.
- **Markup code-behind** — n/a.

## 10. Frameworks & entry points

Four rules apply to OCaml, all single-language and all **detect-only** (no attributes, registrars, base types, methods; `node: true` but nothing is enumerated). Detection keys on `open` and `module-use` imports, so a library used only through qualified names is still seen (CHANGELOG.md:1922-1924; `detect_prefixes.rs:67` writes `open <prefix>` for the synthetic detection test). Fixture f3: all four appear under `seen_no_rules`, `recognised: []`, no entry nodes minted.

| rule id | kind | trigger | rules.rs | gap (abridged) |
| --- | --- | --- | --- | --- |
| `dream` | network | module `Dream` | :6030-6045 | handlers only in argument position; `let () =` has no callable; `@@` pipeline is infix |
| `opium` | network | module `Opium` | :6047-6062 | `App.get "/p" h` threaded with `\|>`; argument position + infix |
| `alcotest` | test | module `Alcotest` | :6064-6079 | `test_case "n" \`Quick f` argument position; `let () = Alcotest.run` no callable |
| `cmdliner` | cli | module `Cmdliner` | :6081-6096 | `Term.const f $ …` argument position; `let () = exit (Cmd.eval cmd)` no callable; `let main` too weak by name |

- **member_sources_for / signature_methods_for / container_attributes_for / markup_kinds_for** — no OCaml entries (grep in rules.rs finds OCaml only at the four specs above).
- **Visibility entries** — none (no visibility signal).
- **Registrar verbs** — none; the plugin ignores `ExtractCtx` (ocaml.rs:27), so user-supplied registrar verbs have no effect for OCaml.
- Corpus apps: `app-ocamlci-alcotest`, `app-ocamlorg-dream`, `app-opam-cmdliner` (`scripts/benchmark.sh:336-338`); language repo `ocaml-dune` (benchmark.sh:89, update-readme-stats.sh:90).

## 11. Test harness entries

- **`alcotest`** is kind `test` (rules.rs:6067) but detect-only: no harness node, no test roots (fixture f3).
- **Naming pattern** — the dead-code pass's default `is_test_name` (`crates/cgg-resolve/src/deadcode/roots.rs:975-987`, `_` arm: `test_*` / `Test*`) applies to OCaml: `let test_thing () = …` → root `builtin:test-name`, `test-entry` (fixture f2). This is a name heuristic, not an Alcotest/ppx_inline_test/ppx_expect rule.
- **ppx_inline_test / ppx_expect** `let%test _ = …`, `let%expect_test "x" = …` — no rule; `let%test _` would produce a `_` callable (by 02, unverified by fixture).
- **OUnit / QCheck / Crowbar (fuzz)** — no rules (grep rules.rs).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** — n/a (`["python","php","ruby"]`, rules.rs:11806).
- **Test-file gating** — none for OCaml.
- **`--no-test-entries`** — no effect (no harness nodes).

## 12. Dead-code analysis

Fixture f2/f7 `--dead-code --dead-code-format json` capability row: `class: degraded`, `visibility/attributes/value_references/dispatch/exports/test_tagging: none`, `max_confidence: medium` (`caps.rs:163-174`: Degraded unless visibility and attributes are declared or observed). So **no OCaml finding is ever `high`**; with the default threshold every OCaml finding is withheld.

Roots that fire for OCaml:

- `builtin:main` — `let main () = …` (`UNIVERSAL_ENTRY`, roots.rs:20; fixture f2). No OCaml entry in `LANG_ENTRY` (roots.rs:23-32).
- `toplevel:invocation` — a call from `let () = run ()` roots `run` (roots.rs:559-608; fixture f7: `run` rooted, `helper` reachable).
- `builtin:test-name` — `test_*` / `Test*` (11).

Not available:

- **Public/exported API** — `.mli` signatures are not read, so every `val`-exported function of a library with no in-tree caller reads as dead (no `lang:export` roots; `FileFacts::exports` unset).
- **Framework roots** — none (10).
- **Value references** — functions passed as arguments (`List.iter f`, `Term.const run`, `test_case … f`) have no edge, so they read as `never-referenced` (fixture f2: `app2`, `lab`, `opt`; f7: `piped` used only via `|>`).
- **Over-extracted nodes** (`_`, plain values, local lets) add findings that are not functions.
- **Caps**: `implements-external-contract`, `macro-generated`, `reflection` never apply. Withheld category seen: `only-used-by-tests … signal not extracted` (fixture f2).
- Evidence on each finding: `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility` (fixture f2 JSON).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/ocaml.txt`** — 116 entries (`wc -l`), loaded at `stdlib.rs:213, 254`: Stdlib modules (`List`, `Array`, `Hashtbl`, `Printf`, `Format`, `String`, `Unix`, `Thread`, `Domain`, `Atomic`, …) and Stdlib functions (`print_endline`, `failwith`, `ignore`, `incr`, `raise`, `string_of_int`, …).
- **Matching** — `List.map`, `Printf.printf`, `String.length` match by first dotted segment (strategy 2, `crates/cgg-core/src/external.rs:20-22`); bare `print_endline` matches exactly. Fixture f1: `List.map` → `stdlib_calls`.
- **`--include-stdlib`** — mints `<stdlib>::Printf.printf`, `<stdlib>::print_endline`, `<stdlib>::String.length` exit nodes, edges at **low** via `stdlib` / `exit-node` (fixture f4).
- **Not in the table**: `Stdlib.Fun` combinators are under `Fun` (present); third-party "standard" libraries (`Base`, `Core`, `Lwt`, `Stdio`) are absent and fall in the external bucket.
- **rust_types-like / CAST_INTRINSICS / CONSTRUCTING_METHODS / implicit packages** — no OCaml entries (n/a).

## 14. Confidence & certainty

- **High**: intra-file single-candidate binds (fixtures f1/f2: `helper`, `loop`, `even ↔ odd`, `inner`, `labeled` — all `high intra-file`). Including the wrong high edge to another function's local `let` (07).
- **Medium**: every cross-file edge seen — `open` / `let open` resolution through step 1c `ScopedSimple` (`cross-file:imports`, fixtures f1, f5). Step 1b (`<prefix>.<name>`, cross_file.rs:5942-5968) could return `module_certain` high, but it needs a callable with qn `B.foo`, which OCaml never produces (02), so cross-file `high` is unreachable in practice (fixture evidence: no cross-file high in any fixture).
- **Low**: `<stdlib>` / `<external>` exit edges under `--include-stdlib` / `--include-external` (fixture f4).
- No `CertainCtx`/`OwnerFacts` path applies (no typed receivers).
- **Mermaid `?`** — every cross-file OCaml edge carries it (fixture f1 `cgg c.ml b.ml`: `N0 -->|?| N5` ×4).
- CHANGELOG.md:1345: OCaml share of edges at `high` went 65 → 77% in WS14.

## 15. Dynamic use & reflection

None. `dynuse` covers Python, JS, Java, C#, Ruby and PHP only (CLAUDE.md; `crates/cgg-lang/src/plugins/dynuse.rs`); the plugin declares no dyn uses. `Obj.magic`, `Dynlink.loadfile`, `Callback.register "name" f` / `Callback.lookup` are not screened (fixture f5 `Obj.magic` is just a stdlib call). No `reflection` or `module-load` cap applies.

## 16. Known gaps & limitations

- **Module qualification never formed** — `extract_module` looks for `module_name` as a direct child of `module_definition`, but it is under `module_binding` (ocaml.rs:127-137); every qn is bare (fixture f1). Consequence: no `M.f` qualified names anywhere.
- **Qualified workspace calls `Mod.f x` never resolve** — the dominant OCaml call form goes to the external bucket (fixture f1: `B.foo`, `B.Sub.deep`, even same-file `M.mfun`). Combined with the above, OCaml cross-file linking only works through `open`.
- **`include M`, local open `M.(e)`, module aliases `module X = M`, functors** — not understood (fixture f5; CHANGELOG.md:1349). README.md:515 overstates "open, include".
- **Pipelines `|>` / `@@` and user operators not recorded** — infix expressions are skipped (fixture f1; rules.rs:6042, 6059).
- **No value references / argument-position functions** — blocks every framework rule (rules.rs gaps) and inflates dead code.
- **`let () = …` has no callable** — OCaml's program entry is invisible; calls inside are `no-enclosing-callable` (fixture f1, f7; rules.rs:6093).
- **Over-extraction** — `let _ =` creates `_` nodes that steal attribution; plain values and local value lets are callables; a parameter can bind at high to another function's local `let` (fixture f6).
- **Objects/classes, methods (`#`), `new`, `external`, operators, `.mli` `val`s** — not extracted / not recorded (fixture f1).
- **No attributes, visibility, exports** → dead-code class `degraded`, max confidence medium (12).
- **No ppx awareness** (`[@@deriving]` generated functions invisible) (08).
- **No ffi** (`external` / `caml_*` C stubs) (09).
- **Framework rules are all detect-only** (dream, opium, alcotest, cmdliner) (10).
- No `#[ignore]`d OCaml tests found (grep `ignore` near the OCaml tests; the five plugin tests, one locals case, two intra-file and two cross-file tests all run).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.ml`/`.mli` by extension only; one grammar for both; no shebang/content/`.mll`/`.mly` |
| 02. Callables extracted | partial | `let`/`let rec`/nested lets; bare qns (module never set); no classes/methods/operators/externals/`.mli`; `_` and values over-extracted |
| 03. Call-site forms | partial | application only; `\|>`/`@@`, `#` methods, operators, values-as-args not recorded |
| 04. Imports & module resolution | minimal | `open`/`let open` at medium; `M.f`, `include`, `M.(e)`, aliases unresolved |
| 05. Type inference & receiver typing | none | no receivers or types recorded |
| 06. Inheritance, dispatch & overloading | none | no classes, no signatures-as-interfaces, not in INHERITANCE/OVERLOAD lists |
| 07. Scoping & shadowing | full | locals, lexical + positional shadowing, `not_def`; one wrong-high bug with local lets |
| 08. Macros, preprocessor & codegen | none | no ppx/cppo; no name_mentions |
| 09. FFI & cross-language links | none | `external` not extracted; no ffi family |
| 10. Frameworks & entry points | minimal | 4 detect-only rules (dream, opium, alcotest, cmdliner); no enumeration |
| 11. Test harness entries | minimal | alcotest detect-only; generic `test_*` name root only |
| 12. Dead-code analysis | minimal | degraded, max medium; roots main / top-level / test-name only; no `.mli` exports |
| 13. Builtins & stdlib tables | full | 116-entry ocaml.txt, dotted-head matching, low exit nodes |
| 14. Confidence & certainty | partial | high intra-file only; cross-file always medium (`?`) |
| 15. Dynamic use & reflection | none | no dynuse coverage |
| 16. Known gaps & limitations | n/a | module qn bug, `M.f` unresolved, infix calls, value refs, `let ()` |
