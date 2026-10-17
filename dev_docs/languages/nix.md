# nix

The Nix plugin is `NixPlugin` in `crates/cgg-lang/src/plugins/nix.rs`. The file is 275 lines and includes 3 inline tests. It parses `.nix` files with the crates.io grammar `tree-sitter-nix`: the workspace pins `"0.3"` (Cargo.toml:119), Cargo.lock:1634-1636 resolves it to 0.3.0, and nix.rs:25-27 binds `tree_sitter_nix::LANGUAGE`. Nothing is vendored.

A single recursive `NixWalker` (nix.rs:47-217) does three things:

- It records a `binding` whose right-hand side is a `function_expression` as a `FreeFunction`. The binding's raw attrpath text is the name.
- It records every `apply_expression` whose callee is a `variable_expression` or a `select_expression` as a `RefRecord`.
- It turns the builtin `import X` into an `ImportRecord` of kind `"import"`.

The plugin overrides none of these trait methods, so it gets the defaults:

- `signals()`: every optional signal is false (lib.rs:304-306).
- `shebangs()`: none (lib.rs:298-300).
- `max_line_bytes()`: none (lib.rs:313-315).
- `ts_language_for()`: the default (lib.rs:322-324).

The plugin also ignores `ExtractCtx` (`_ctx`, nix.rs:31). The trait has no `resolver_kind` method, so the generic pipeline resolves Nix. The only Nix-specific resolver code is the `"dart" | "solidity" | "nix"` file-relative import arm in `cross_file.rs:2029-2060`.

Nix is registered at plugins.rs:160. Dead code classifies it as `SCRIPT_DRIVEN` (`crates/cgg-resolve/src/deadcode/caps.rs:27-36`). Its single framework rule `nix` (rules.rs:6679-6695) is a gap-only rule.

Fixtures are under `<scratch>/parity/nix/`, as directories f1 … f4. Each was run with `target/release/cgg` 0.11.0 using `-t json -o <out>.json`. The graph was dumped with `show.py` and the audit sidecar with inline Python.

## 01. Detection & parsing

- **Extension `.nix` only** (nix.rs:22-24). Evidence: every fixture file reports `detected_via: extension:.nix`.
- **No shebangs** (default `&[]`).
  - f4/`script`, starting `#!/usr/bin/env nix-shell` / `#! nix-shell -i bash`, is detected as **bash** via `shebang:sh`. The bash `sh` needle is a substring of `nix-shell`.
  - In practice that is right for `nix-shell -i bash` scripts, but it is accidental: Nix itself has no shebang detection.
- **No content detection.** detect.rs has no Nix branch (grep for `nix` in `crates/cgg-lang/src/detect.rs` finds nothing).
- **Single grammar.** There are no dialects, no `ts_language_for`, and no re-parse or fallback on error.
  - A file with a syntax error is still extracted from tree-sitter's error-recovered tree, and `parse_status` is `error`.
  - In f4/a.nix, `broken = x: (( x;` set `parse_status: error`. The `broken` binding was lost, but every other binding in the file was still recorded.
- **Skip conditions.** Only the generic ones apply: binary, size and walk deny list. The tree-depth guard `MAX_TREE_DEPTH` = 4000 (`crates/cgg/src/lib.rs:155`, checked at :451) skips a file as `TooDeep`. Not separately exercised for Nix.
  - There is no `max_line_bytes` cap (default `None`).
  - The minified-file rule (lines over 2,000 bytes) only gates `locals`, which Nix does not collect.
- **Curried-application depth guard.** `record_call` descends at most 64 nested `apply_expression` levels to find the callee (nix.rs:155-163).
- Framework detection gets one Nix test: `crates/cgg/tests/detect_prefixes.rs:88` (`every_detect_prefix_can_actually_fire`) writes `{ }: import <prefix>` into a `.nix` file to prove each detect prefix can fire.

## 02. Callables extracted

- **Function-valued binding** in a `let`, an attrset or a `rec` attrset, for example `greet = name: "Hi ${name}";`.
  - Becomes a `FreeFunction`. `simple_name` and `qualified_name` are both the raw attrpath text (nix.rs:126-151).
  - The span is the whole binding. `signature_hint` comes from `extract_signature` over the binding text.
  - Evidence: unit test `function_bindings_are_defs` (nix.rs:239-252); f1 `greet`, `add`.
- **Curried function** `add = a: b: a + b;` is one callable; the inner lambdas are not separate (f1 `add`).
- **Pattern-argument lambda** `pat = { a, b ? 2, ... }@args: a + b;` is a callable.
  - Its `signature_hint` is truncated to `pat`, because `extract_signature` stops at the first depth-0 `{` (plugins.rs:74-86) (f1).
- **`rec { inner = x: outer x; outer = y: y; }`**: both members are callables, and `inner -> outer` is an intra-file high edge (f1).
- **Nested attrpath binding** `attrs.nested.fn = z: z;` is named `attrs.nested.fn` for both simple and qualified name (f1, f4). Because the simple name contains dots, a call through the attrpath does not match it by name (see section 03).
- **Quoted attribute name** `"quoted-name" = x: x;` keeps the quotes in its name: `"\"quoted-name\""` (f3/mod.nix).
- **Flake `outputs` lambda** `outputs = { self, nixpkgs }: ...` is a callable spanning the whole outputs body (f3/flake.nix:4-9).
- **Overlay** `overlays.default = final: prev: {...}` is a callable named `overlays.default` (f3).
- **Nested definitions.** Lambdas bound inside another callable's `let` (for example `mk` inside `outputs`, or `inner` inside `outer`) are callables too.
  - The qualified name is **not** qualified by the enclosing binding: `mk`, not `outputs.mk` (f3). Two `inner`s in different functions get the same qualified name `inner` (f2).
- **No owner.** Every def is a `FreeFunction` with empty visibility and attributes (nix.rs:138-150). An attrset is never an owner, so methods, constructors, properties, operators and overloads do not apply.
- **Not extracted:**
  - A file whose top-level expression is a lambda, such as `{ config, lib, pkgs, ... }: { ... }` or `{ stdenv }: stdenv.mkDerivation {...}`. It is not a `binding`, so the module/package function itself is never a node (f3/mod.nix, f3/pkg.nix: no callable for the file lambda). The `nix` rule's gap text says the same (rules.rs:6692).
  - An anonymous lambda in argument position, such as `(n: v: v)` in `mapAttrs (n: v: v) {}` (f1: no callable).
  - `inherit f;` / `inherit (x) f;`: not a `binding` node, so nothing is extracted (code: nix.rs:73 matches only `binding`).
  - A binding whose value is a call returning a function, for example `f = lib.makeOverridable g;`. The right-hand side is not a `function_expression` (nix.rs:74-76).
  - A binding lost to error recovery (f4 `broken`).

## 03. Call-site forms

- **Bare application** `f 42` is a `RefRecord` with `name=f` and an empty receiver (nix.rs:165-171, 207-213). Unit test `apply_is_a_call`; f2 `greet x` → `main -> greet` high.
- **Curried application** `add 1 2`: `record_call` walks left through nested `apply_expression`s to the callee (nix.rs:155-163).
  - The walker then also descends into the inner `apply_expression` and records the same callee **again**, at the same `site_byte`.
  - The audit shows the duplicates: f1 line 14 `add` ×2, f3 `lib.mkIf` ×2, `builtins.map` ×2. Edges are deduplicated: f2 `main -> add` is one edge.
- **Select (path) application** `helpers.double 3`: the name is the text after the last `.` and the receiver is the prefix (`helpers`, `pkgs.lib`, `attrs.nested`) (nix.rs:172-180). Evidence: f1 audit receiver hints.
- **Parenthesised / computed callee** is **not recorded**, because the callee kind is neither variable nor select (nix.rs:181). Examples:
  - `(x: x) 1` (f1 r9, f2 `e`);
  - `(if z then inner else helper) z` (f3 user8).
- **Pipe operators** `z |> inner` are not `apply_expression`s, so no call is recorded (f3 user4: no edge).
- **Calls inside string interpolation** `"${toString (inner z)}"` are recorded (f3 user3 → `inner` high).
- **Calls after `assert` and inside `with`** are recorded (f3 user5 and user6 → `inner`).
- **Functions passed as values** (`map helper [ z ]`, `builtins.map greet [...]`) are **not recorded**. Only the applied `map` is a call; `greet`/`helper` produce no value ref (f3 user: no edge to `helper`). `signals().value_refs` is false.
- **Explicit generics, constructors, method refs, paren-less calls, registrar arguments, arity:** not applicable or not recorded. `RefRecord::arity` is never set (nix.rs:207-213).
- **`import X`** is consumed as an import record, never a call (nix.rs:187-205).
- **`builtins.getAttr "inner" self`** is a call to `getAttr` with receiver `builtins`. The string is not read (f3 user2).
- **Top-level calls** (the `in` body of a file-level `let`, or the attrset values of a file that is not inside a function binding) have no enclosing callable. They are unresolved `no-enclosing-callable` (f1 lines 13/14/22, f2 line 23). They still serve as dead-code `toplevel:invocation` roots (section 12).

## 04. Imports & module resolution

- **`import ./path.nix`, `import ./dir`, `import <nixpkgs>`** each become `ImportRecord { kind: "import", alias: "" }`.
  - The path is the argument text with `<`, `>` and `"` trimmed (nix.rs:187-203). Unit test `import_captured` (`<nixpkgs>` → `nixpkgs`).
- **Resolution** goes through the `"dart" | "solidity" | "nix"` arm (cross_file.rs:2029-2060).
  - It tries the cleaned path plus `.sol`/`.dart`/`.nix` suffixes in `collect_include_defs`, with depth budget 4.
  - Paths are normalised lexically, so `../` works (CHANGELOG.md:1945-1952).
  - The imported files' definitions become "direct imports" for bare-name lookup.
- **The receiver is not tied to the imported file.**
  - The alias is always empty (nix.rs:199), so `helpers = import ./lib/helpers.nix; ... helpers.double x` is not a lookup inside that file.
  - In f2, `helpers.double x` bound **medium** to `double` in **both** `lib/helpers.nix` and `lib/helpers/default.nix`. `h2.double` behaved the same.
  - A bare `double x`, which in Nix is not in scope without `with`, bound **high** to the imported file's `double` (f2 line 7, `cross-file:imports`). That is a false-high edge.
- **Directory import.** `import ./lib/helpers` resolved to `lib/helpers.nix` (by the `.nix` suffix try), not to `lib/helpers/default.nix`. `default.nix` is never tried; there is no such suffix in cross_file.rs:2034-2039.
- **Non-imported files are reached by name.** `triple x` bound medium to `lib/other.nix`, which was never imported (f2 line 15). `set.go q` bound medium to `go` in both a.nix and the unrelated b.nix (f4).
- **`callPackage ./pkg.nix {}`** is an ordinary call (`pkgs.callPackage`, external), not an import (f1 r13, f3). The rule gap text says the same (rules.rs:6692).
- **NixOS `imports = [ ./sub.nix ]`** is a list-valued binding and is not an import record (f3/mod.nix; no code reads `imports`).
- **`inherit (helpers) double; in double 9`** and **`with helpers; double 8`** do not bring names into scope. Both are unresolved `no-candidate-in-file` (f1 lines 26/27).
- **Not supported:**
  - `FileFacts::packages`;
  - manifest reading (`flake.nix` inputs, `flake.lock`);
  - `heads.rs` external-module refusal (Nix is not in `WorkspaceVocab`);
  - `modules.rs` `not-in-imported-module` refusal;
  - flake inputs (`nixpkgs.legacyPackages...`).

  A grep for `nix` in heads.rs and modules.rs finds nothing.
- **`import <nixpkgs> {}` path (unverified).** For the outer `apply_expression` of `(import <nixpkgs>) {}`, nix.rs:189 takes `node.child(1)` of the *outer* node, which is `{}`. The walk then visits the inner apply and records `nixpkgs` correctly (unit test `import_captured`). So a spurious import with path `{}` is probably also recorded. This was not observable in output.

## 05. Type inference & receiver typing

- **None.** Nix is dynamically typed, and the plugin sets no `typed_receiver`, `arg_type`, `local_types` or `scoped_types`. There is no Nix entry in `type_hints.rs`, `closure_types.rs` or `receiver.rs` (grep finds no `nix`).
- The receiver is only a textual hint (the select prefix). Intra-file binding ignores it entirely: `self.inner z` bound **high** to the file's `inner` (f3 user9), and `set.go q` matched every `go` (f4).
- Not applicable: casts, constructions, `declared_type` wrappers, implicit receivers, weak receiver types.

## 06. Inheritance, dispatch & overloading

- **Not applicable.** Nix has no classes, so no `ClassDecl`, bases, `super`, `member_of_calling_class` or `overrides_below`.
- `nix` is not in:
  - `INHERITANCE_LANGS` (dispatch.rs:110);
  - `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583, `["java", "scala", "cpp"]`).
- Nix has no trait impls (`signals().impls` false), so `--dynamic-dispatch` adds nothing.
- Overlay `final: prev:` override chains, `//` attrset merges, `lib.extends`/`lib.fix` fixpoints and the NixOS module system's option merging are not modelled.
- **Same-name fan-out still happens** through generic by-name lookup. For example, f2 `inner` bound to both nested `inner`s, and f4 `go` bound to two files. This fan-out is bounded by `--fanout-cap`.

## 07. Scoping & shadowing

- **No `FileFacts::locals`.** `locals::spec` has no `"nix"` arm (`crates/cgg-lang/src/locals.rs:380-393`), so nothing is ever `shadowed-by-local`.
  - Evidence: f1 `shadow = greet: greet "x";` produced `shadow -> greet` at **high** (intra-file) to the file-level `greet` binding, although `greet` there is the lambda parameter.
- **No lexical narrowing of nested definitions.** Nix is not in the CLAUDE.md lexical-narrowing list.
  - f2: `outer = x: let inner = y: y; in inner x;` and `other = x: let inner = z: z+1; in inner x;` each bound to **both** `inner`s (medium, `cross-file:imports`), i.e. four edges.
- **`let`, `rec` and `with` scoping are not modelled.**
  - Calls to `rec` siblings work only because they share the file (f1 `inner -> outer`).
  - `with lib; mkDefault ...` leaves `mkDefault` external (f3 user6).
- **No positional shadowing and no conditional definitions** (`DefRecord::conditional` unset).

## 08. Macros, preprocessor & codegen

- **Not applicable.** Nix has no macro or preprocessor system, so nothing is ever tagged `macro:<name>`.
- `name_mentions` is never populated (nix.rs never touches it).
- Derivation builders, string-embedded shell code in `buildPhase` and so on, and generated `.nix` (for example `node2nix` output) are parsed as ordinary Nix where present. Shell inside strings is not analyzed.

## 09. FFI & cross-language links

- **None.** `ffi.rs` and `descriptor.rs` have no Nix handling (grep finds no `nix`), and Nix has no markup code-behind.
- Builder scripts (`builder = ./build.sh`) and `runCommand "x" {} ''...''` shell strings are not linked to the bash files they name or embed.

## 10. Frameworks & entry points

- **Rule `nix`** (rules.rs:6679-6695): `language: "nix"`, `kind: TrustKind::Lifecycle`.
  - `detect: ["nixpkgs", "nixos", "home-manager", "flake-utils", "."]`.
  - `node: true`, with no attributes, registrars, base_types, methods, `detect_calls` or `detect_paths`. The gap text explains why entries cannot be enumerated.
  - Because the rule has no matchers it is reported only under "seen, no rules" (frameworks/mod.rs:388-416), and it **never mints an entry node**.
  - The `"."` prefix matches every relative import, via `import_matches`, which accepts a remainder starting with `/` (frameworks/mod.rs:492-509). So any Nix file containing `import ./x.nix` triggers the coverage line (f1 stderr: "seen, no rules nix — found in 1 file(s)").
- No other rule in `SPECS` names `nix`: grep for `language: "nix"` gives only line 6681. `benchmark.sh` lists `~nix` as an unverified or absent app on `app-jabref-javafx` and `haskell-pandoc` (benchmark.sh:206, 220).
- **Not covered:**
  - the `outputs` lambda (it *is* a callable, but has no entry rule);
  - NixOS/home-manager module functions (not callables at all);
  - overlays;
  - `callPackage` targets;
  - `nixosConfigurations`/`devShells`.
- No `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for` or visibility entries apply.
- Registrar verbs (`ExtractCtx`) are ignored (nix.rs:31).

## 11. Test harness entries

- **None.** There is no `kind: test` rule for Nix: no `nixosTest`/`runNixOSTest`, `lib.runTests` or `nix-unit` harness. The only Nix rule is the Lifecycle gap rule.
- `TEST_CLASS_MEMBERS_ARE_ROOTS` does not apply. `signals().test_defs` is false, so the dead-code `test_tagging` capability is `none` (f2 `--dead-code-format json` capabilities).

## 12. Dead-code analysis

- **Class `script-driven`.** Nix is in `SCRIPT_DRIVEN` (caps.rs:27-36, assigned at caps.rs:165-166).
  - The evidence `language-is-script-driven` caps every finding at **low** (`crates/cgg-core/src/deadcode.rs:314-316`).
  - The capability row reports `max_confidence: "medium"`.
  - Categories that depend on roots are **withheld** with reason "script-driven language" (`crates/cgg-resolve/src/deadcode/mod.rs:811-812`).
  - f2: 3 findings, all `low`, 0 shown at the default high threshold. `reachable-only-from-dead-code` had 2 withheld.
- **Capabilities.** All are `none`: visibility, attributes, value_references, dispatch, exports, test_tagging (f2 JSON `capabilities`).
- **Roots.** Only `toplevel:invocation` (roots.rs:585-612): a top-level call site whose name matches a callable roots that callable.
  - f2: `main` rooted, "called from module top level at f2/default.nix:23".
  - The match is by `(language, simple_name)`, so it would root every same-named callable.
- **No public or exported roots.** Attrset members a file returns are effectively its exports, but nothing marks them.
- **Unresolved-doubt handling** is generic. `no-unresolved-site-with-name` evidence is emitted. Value-position uses (`map helper`) are invisible, so a function used only as a value is reported (low).
- Not applicable to Nix: the `implements-external-contract`, `macro-generated` and `reflection` caps.

## 13. Builtins & stdlib tables

- **There is no `stdlib/nix.txt`.** `stdlib_names` has no `"nix"` entry (`crates/cgg-core/src/stdlib.rs:214-250`).
  - Every `builtins.*` call, every bare builtin (`map`, `toString`, `throw`, `abort`) and every nixpkgs `lib.*` call is therefore classified **external** (`name_screen_applied: "external"`), never stdlib. This follows `classify_one` rule 1 or 2 in `crates/cgg-core/src/external.rs:294-305`.
  - Evidence: f1/f2/f3 audit `external_calls` for `builtins.map`, `builtins.trace`, `toString`, `map`, `lib.mkIf`.
  - The stderr summary always says `0 stdlib`.
- **`--include-stdlib`** therefore mints nothing for Nix.
- **`--include-external`** mints `<external>::…` exit nodes at `low`. f4 produced `<external>::undefinedThing` and `<external>::attrs.nested::fn`.
- There is no rust_types-like table, no `CAST_INTRINSICS`/`CONSTRUCTING_METHODS` entry and no implicit packages.
- Shadowing risk: because no Nix stdlib exists, a project function named `map` would capture every bare `map` call by name (generic by-name behaviour; unverified for Nix specifically).

## 14. Confidence & certainty

- **High** is reached by:
  - intra-file unique-name binding (f1 `inner -> outer`, f2 `main -> greet`, f3 `helper -> inner`);
  - a unique name in an imported file (f2 `main -> double`/`quad`, `cross-file:imports`, high).

  None of these is semantically certain for Nix, because scoping is not modelled. False highs observed:
  - a parameter-shadowed name (f1 `shadow -> greet`);
  - an unbound receiver (f3 `self.inner` → `inner`);
  - a bare name not in scope (f2 `double` reached only through `helpers.`).
- **Medium**: by-name fan-out to several candidates and non-imported files (f2 `helpers.double` ×2, `triple`, the `inner`s; f4 `go` ×2).
- **Low**: only `--include-external` exit nodes and dead-code findings.
- **Mermaid `?` marker** appears on arrows with no high site: `N1 -->|?| N10`, `N3 -->|?| N4` (f2 mermaid output) and `N3 -->|?| N1` (f4).
- **Inconsistency observed (f4).** `caller = q: attrs.nested.fn q` produced both a **high** cross-file edge to `attrs.nested.fn` and an `<external>::attrs.nested::fn` exit node for the same site. The audit lists it under `external_calls` too. The high edge probably comes from name+receiver re-joining in cross_file (unverified mechanism).

## 15. Dynamic use & reflection

- **None.** The `dynuse` signal is off (default `signals()`), and `plugins/dynuse.rs` covers only Python/JS/Java/C#/Ruby/PHP.
- These are all plain external calls whose string or path arguments are not read (f3 user2 `getAttr` external):
  - `builtins.getAttr "inner" self` and `self.${name}` dynamic attribute selection;
  - `builtins.import` of a computed path, `import (./. + "/${x}.nix")`, `scopedImport`;
  - `callPackage`.
- No module-load screen is applied to Nix.

## 16. Known gaps & limitations

- **Gap text** `rules::gap_for("nix", "nix")` (rules.rs:6692) says:
  - there are no entry rules;
  - flake `outputs`, module lambdas, overlays and `callPackage` targets are the real entry points;
  - a file whose body is one lambda contributes no callable;
  - `callPackage` is not an import.
- **README** language table row (README.md:521): `Nix | import <path> | — | function-valued bindings, apply expressions`. Benchmark row: home-manager, 1,072 callables, 1,162 edges, 30% (README.md:1348; benchmark.sh:96 `nix-home-manager`).
- **Found while cataloguing (fixtures):**
  1. Top-level module/package lambdas are not callables, so most NixOS modules and packages have no node (f3).
  2. Calls outside any function binding are `no-enclosing-callable` (f1, f2). In a typical `default.nix`, most call sites are therefore unattributed.
  3. Lambda parameters do not shadow, and nested `let` definitions are not narrowed. This produces false **high** edges (f1 `shadow -> greet`) and cross-function fan-out (f2 `inner`).
  4. The import binding (`h = import ./x.nix`) is not linked to the receiver `h.f`. Bare names from imported files bind high, and non-imported files are reached by name (f2, f4).
  5. `import ./dir` does not try `./dir/default.nix` (cross_file.rs:2034-2039).
  6. `inherit`, `inherit (x) …` and `with x;` neither define nor import names (f1 lines 26/27).
  7. Curried applications are recorded once per `apply_expression` level, so the audit counts duplicates (f1, f3). Parenthesised or computed callees and `|>` pipes are dropped (f1 r9, f3 user4/user8).
  8. Value references (`map f xs`) are not recorded.
  9. Attrpath names: `a.b.f = …` keeps its dots as the simple name, and quoted names keep their quotes (f3 `"quoted-name"`). Qualified names carry no enclosing-binding context, so same-named nested lambdas collide.
  10. There is no stdlib table, so all `builtins` and `lib` calls are "external".
  11. There are no test harness rules (`nixosTest`, `lib.runTests`).
  12. There are no shebang needles. A `nix-shell` script is detected as bash only by the `sh` substring accident (f4/script).
  13. Possible spurious `{}` import record for `import <p> {}` (unverified, nix.rs:189).
- No `#[ignore]` tests for Nix were found. The only tests are the 3 inline tests in nix.rs:239-274 and `detect_prefixes.rs:88`.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.nix` only; no shebang or content detection; error-recovered trees extracted |
| 02 Callables extracted | partial | only function-valued bindings; file-level module/package lambdas and anonymous lambdas are not nodes; flat names |
| 03 Call-site forms | partial | bare and select applications; curried calls are duplicated; parenthesised callees, pipes and value refs are dropped |
| 04 Imports & module resolution | minimal | `import path` gives file-relative include defs; no alias↔receiver link, no `default.nix`, no `inherit`/`with`/`callPackage` |
| 05 Type inference & receiver typing | none | receiver is a textual hint only and is ignored intra-file |
| 06 Inheritance, dispatch & overloading | n/a | no classes; overlays and fixpoints not modelled; not in INHERITANCE or OVERLOAD lists |
| 07 Scoping & shadowing | none | no locals table; parameters do not shadow (false high); no lexical narrowing |
| 08 Macros, preprocessor & codegen | n/a | language has none; `name_mentions` unused |
| 09 FFI & cross-language links | none | builder scripts and embedded shell are not linked |
| 10 Frameworks & entry points | minimal | one gap-only Lifecycle rule `nix`; coverage line only, no entry nodes |
| 11 Test harness entries | none | no `nixosTest` / `runTests` rules |
| 12 Dead-code analysis | minimal | script-driven: findings capped low, root-dependent ones withheld; only `toplevel:invocation` roots |
| 13 Builtins & stdlib tables | none | no `stdlib/nix.txt`; all builtins and `lib` calls are external |
| 14 Confidence & certainty | partial | high from unique-name intra-file and import paths, but scoping-blind (false highs); `?` marker works |
| 15 Dynamic use & reflection | none | `getAttr`, `${}` selection and computed imports are not screened |
| 16 Known gaps & limitations | n/a | 13 gaps listed above; gap text at rules.rs:6692 |
