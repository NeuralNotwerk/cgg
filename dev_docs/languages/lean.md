# lean

The Lean 4 plugin (`LeanPlugin`, id `"lean"`, `crates/cgg-lang/src/plugins/lean.rs`, 909 lines) is a best-effort *syntactic* extractor for Lean 4 sources. It produces declarations (`def`/`theorem`/`lemma`/`abbrev`/`instance`/`axiom`/`opaque`/`constant`/`example`), structures/classes with their fields, and inductives with their constructors. It records function application, dot-projection references, and the lemma references inside tactic blocks (`exact`/`apply`/`simp [..]`/`simp only [..]`/`rw [..]`/`rewrite [..]`/`unfold`), plus `import`/`open` records. The grammar is vendored rather than taken from crates.io: `crates/cgg-lang/vendor/lean4/{parser.c,scanner.c,tree_sitter/}` from `wvhulle/tree-sitter-lean` rev `79a84684…` via Nathan Howell's fork (`vendor/lean4/PROVENANCE.md`). It is compiled in `crates/cgg-lang/build.rs:40-49` and bound through `tree_sitter_language::LanguageFn::from_raw(tree_sitter_lean)` (`lean.rs:43-50`), because the published `tree-sitter-lean4` crate has `links = "tree-sitter"` and cannot coexist with the workspace runtime. Registered at `crates/cgg-lang/src/plugins.rs:169`. The trait has no `resolver_kind` method. Lean declares **no** `signals()`, so it gets `PluginSignals::default()` with every bit false (`crates/cgg-lang/src/lib.rs:304-306`). On the resolver side it uses the generic "dotted module, unqualified import" branch of `cross_file` (`crates/cgg-resolve/src/cross_file.rs:2064-2080`) and no language-specific resolver. The module docs list what the plugin does not attempt: typeclass-method dispatch, macro/`notation` expansion, and type-directed dot resolution, all of which need the Lean elaborator (`lean.rs:22-26`). It came in with the 0.10.0 Lean contribution (CHANGELOG.md:1835-1849).

## 01. Detection & parsing

- **Extension `.lean`**: the plugin's only detection key (`lean.rs:59-61`). Fixture audit: `"detected_via": "extension:.lean"`.

  ```lean
  -- any file named Foo.lean
  ```

- **Shebangs**: none. `shebangs()` is not overridden, so it is the default `&[]` (`crates/cgg-lang/src/lib.rs:298-300`).
- **Content detection**: none. There is no Lean entry in `cgg-lang::detect`: `grep lean crates/cgg-lang/src/detect.rs` returns nothing.
- **Grammar**: one grammar, the vendored `tree_sitter_lean` (`lean.rs:43-50`, `build.rs:40-46`). `ts_language_for` is not overridden, so every file uses the same grammar. The CHANGELOG says the grammar ran clean under ASan/UBSan on 4,766 inputs (CHANGELOG.md:1847-1848). Memory notes `parser.c` is about 44 MB (`PROVENANCE.md`).
- **Error recovery**: there is no re-parse or fallback. A file with tree-sitter errors is still walked, with `parse_status: "error"` in the audit. Fixture results:
  - `where` clauses and `instance (priority := low) …` produce `error` parses.
  - `macro "x" : tactic => …` and `mutual … end` also produce `error`, although the `mutual` defs were still extracted.
  - Fixture runs: `iso/where`, `iso/prio`, `iso/macro`, `iso/mutual` under the scratch dir.
- **Skip conditions**: only the generic ones apply.
  - `MAX_TREE_DEPTH` = 4000 → `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155, 451-454`).
  - `max_line_bytes()` is not overridden, so it is `None` and there is no `LongLine` skip (`lib.rs:313-315`, `crates/cgg/src/lib.rs:433-439`).
  - There is no minified-file handling, because Lean records no `FileFacts::locals`, which is where the 2,000-byte minified rule lives.
- **Unicode safety**: the `visibility()` scan is capped at 48 bytes and snapped to a char boundary (`lean.rs:321-341`). Test `unicode_signature_does_not_panic` (`lean.rs:893-908`).
- **Not supported**: `.olean`/`.ilean` build artifacts (they are not source), and `lakefile.lean`-specific handling. `lakefile.lean` is parsed as ordinary Lean; there is no manifest reading (unverified that anything special happens; there is no `manifest.rs` entry).

## 02. Callables extracted

All callables are `DefVariant::FreeFunction`, with the exceptions of `Property` (structure fields) and `Constructor` (inductive constructors) (`lean.rs:391, 438, 445, 459, 466-472`).

- **`def` / `theorem` / `lemma` / `abbrev`**: node kind `definition` → `extract_definition` (`lean.rs:242-244, 343-404`).

  ```lean
  def double (n : Nat) : Nat := n + n
  @[simp] theorem double_eq (n : Nat) : double n = n + n := rfl
  abbrev MyNat := Nat
  ```

  Each becomes a `function` node. Fixture `proj`: `Demo.double`, `Demo.double_eq`, `Demo.MyNat`.
- **`axiom` / `opaque` / `constant`**: separate grammar kinds, handled by the same arm (`lean.rs:237-244`). Test `axiom_opaque_and_constant_are_definitions` (`lean.rs:660-698`).
- **`partial def` / `noncomputable def` / `unsafe def`**: extracted (fixture `p2`: `loop`, `nc`, `us`).
- **`instance`**:
  - A *named* instance (`instance instAddPoint : Add Point where …`) becomes a function node tagged with attribute `"instance"` (`lean.rs:356-361, 393-395`). Test `attributes_and_instance_marker_are_captured`.
  - An *anonymous* instance (`instance : Inhabited Point := …`, `instance [Ord β] : …`) is **not** a callable. Lean.rs:362-372 refuses to name it after its body, and test `a_proof_term_is_not_part_of_the_declaration_name` (`lean.rs:592-608`) covers this. Consequence: its body references have no enclosing callable (`no-enclosing-callable` in the fixture `proj` audit, line 40).
  - `instance (priority := low) instX : …` fails to parse and yields **no** callable (fixture `iso/prio`).
- **`example`**: `record(node, false)` only, with no callable (`lean.rs:247`). Its references become `no-enclosing-callable` (fixture `iso/example`).
- **`structure` / `class`**: the type becomes a function node, and each `structure_field` becomes a `Property` node with qualified name `Type.field` (`lean.rs:431-450`). Test `structures_and_inductives`. Fixture: `class Shape … area : α → Nat` → `Shape` + `Shape.area` (property). A `class` is parsed as a structure.
- **`inductive` / `class inductive`**: the type becomes a function node, and each `constructor` becomes a `Constructor` node `Type.ctor` (`lean.rs:246, 452-477`). Fixture `p2`: `Dec`, `Dec.yes`.
- **Dotted declaration names**: `def Point.norm`, `axiom Cert.sosDeg10`, `def Foo.Bar.baz`.
  - The first dotted group of the direct identifier children becomes the name (`lean.rs:373-392`).
  - `simple_name` is the last segment and `qualified_name` is the namespace prefix plus the full dotted name (`lean.rs:388-389`).
  - Fixture: `Demo.Point.norm` (simple `norm`).
  - The fix for the "….rfl" naming bug (only an exact `.` gap joins identifiers) is at `lean.rs:194-203`.
- **Namespaces**: `namespace X.Y … end` pushes onto and pops an explicit stack. `section` pushes an empty frame (`lean.rs:232-236`, `161-177`). Test `definitions_are_namespaced`.
- **Qualified names / owners**: `ns_prefix + dotted` (`lean.rs:175-177`). Owners are recovered on the resolver side through `names::owner_from_qn` on the dotted qualified name (comment at `lean.rs:379-387`). No `owner`/class records are set in `DefRecord`.
- **Signature hint**: the text up to `:=` or `where` at bracket depth 0, with whitespace collapsed (`lean.rs:112-137`). Fixture: `"def double (n : Nat) : Nat"`.
- **Mutual blocks**: `mutual def isEven … def isOdd … end` extracts both defs, with the mutual-recursion edges at `high` (fixture `proj`, `iso/mutual`).
- **Not supported**:
  - **`where` auxiliary definitions** (`def w … := aux n where aux k := …`): `aux` is not extracted and the call to `aux` goes to the external bucket (fixture `iso/where`, `proj` line 34).
  - **`let rec go …`**: not a callable; `go` calls are external (fixture `iso/letrec`).
  - **`fun x => …` lambdas**: not callables. Their bodies are attributed to the enclosing def (fixture `p2` `lam -> A.foo`).
  - **Instance method bodies** (`add a b := …` inside `where`): not callables. They are attributed to the named instance, or lost (no enclosing callable) for an anonymous instance (fixtures `proj`, `iso/classm`).
  - **`open X in def …`**: the def is **not extracted at all** (fixture `iso/openin`: DEFS only `foo`).
  - **`macro`, `macro_rules`, `syntax`, `elab`, `notation`, `infix`**: no callable (fixtures `iso/notation`, `iso/macro`, `p2`).
  - **`deriving` handlers**: no generated callable (fixture `iso/deriving`).
  - **Structure `extends`**: no base record. Inherited fields are not reachable (§06).
  - **Overloads / pattern-matching equations**: `def m : C → Nat | .red => … | .green => …` is one callable. There is no clause model, and Lean is not in `is_clause_language` (`crates/cgg-resolve/src/intra_file.rs:857-859`).

## 03. Call-site forms

References are recorded only by `record()` (`lean.rs:497-572`). In term mode (`bare_ok = false`) a standalone identifier is **not** a reference. Only the head of an `application`, the last identifier of a `projection`, and tactic operands are recorded.

- **Application `f x y`**: the head identifier is recorded as a bare reference (`lean.rs:521-540`). Test `application_and_projection_calls`. Fixture `iso/plain`: `foo n + foo (foo n)` gives 3 edges at high.
- **Projection / dot call `p.x`, `Demo.prot 3`, `(foo 1).succ`**:
  - The last identifier is the name and the collapsed text of the first child is `receiver_hint` (`lean.rs:541-553`).
  - Fixture: `p.norm` → `Demo.Point.norm` high, and `Demo.prot` → `Demo.prot` high.
  - `(foo 1).succ` records `succ` with receiver `(foo 1)`, which goes to external.
- **Qualified/path call `A.go n`, `Foo.Bar.baz`**: a projection with receiver `A`. It binds through the qualified name (`iso/ns_dotted`, `imp/qualA`: `c -> A.go` high).
  - Side effect: a multi-segment path also records the intermediate segment as a reference (`Bar` with receiver `Foo`, unresolved `no-candidate-in-file`; fixture `iso/ns_dotted`).
- **Tactic operands** (`exact foo`, `apply foo`, `unfold foo`, `simp [a, b]`, `simp only [a, Set.mem_def]`, `rw [a, ← b]`, `rewrite [X.y] at h`):
  - In `tactic_apply`, the first identifier is the tactic name and every other operand is recorded in bare mode (`lean.rs:554-565`).
  - `tactic_config` (the `[…]` list) forces bare mode (`lean.rs:510-520`).
  - The modifiers `only`/`with`/`at`/`using`/`from`/`generalizing` are filtered out (`lean.rs:100-105`).
  - Tests: `tactic_references_are_captured`, `rewrite_lemmas_are_captured`, `simp_only_lemmas_are_captured`.
  - Fixture `iso`: `simp only [foo]`, `unfold foo` and `exact` each bind `th -> foo`.
- **Anonymous-constructor arguments `⟨foo 1, foo 2⟩`**: the applications inside are recorded (fixture `iso/anonctor`).
- **String interpolation `s!"{foo 1}"`**: recorded (fixture `iso/interp`: `m -> foo` high).
- **Binder self-references**: a name bound by the declaration's own `explicit_binder`/`implicit_binder`/`instance_binder` is never recorded unqualified (`lean.rs:150-153, 396-429, 485-487`). Test `a_binder_name_is_the_local_not_a_declaration_elsewhere`.
- **Type annotations recorded as calls (false references)**: binder/return types that parse as applications or projections are recorded as call sites.
  - `(xs : List Nat)` → reference `List`; `Add Point` → `Add`; `IO Unit` → `IO` (fixtures `proj`, `iso/gen`).
  - A workspace type in an annotation becomes an edge: `def useIt (p : Demo.Point)` → `useIt -> Demo.Point` high. `instance instSh : Sh Nat` → `instSh -> Sh` high (fixtures `proj`, `iso/classm`).
- **Not recorded at all** (no edge and no audit record):
  - Pipe operators: `foo <| foo n` (`iso/pipe1`, which records nothing, not even the inner `foo n`), `n |> foo` (`iso/pipe2`) and `foo $ n` (`iso/dollar`).
  - Explicit-argument `@foo 5` (`iso/at`).
  - A function passed as a value (`xs.map foo`, `def bareRef := foo`) (`iso/bareref`, `p3` `asArg`). The callee in argument position is a bare identifier in term mode.
  - `#eval foo 3` / `#check nc` (`iso/eval`: no reference).
  - `|>.` method chains (`xs.map foo |>.filter …`): only `map` (receiver `List Nat`) is recorded (`iso/dotpipe`).
- **Leading-dot constructor `.red`**: recorded with receiver_hint `"."` and never bound. It is unresolved `candidates-in-other-files` and goes to the external bucket even when `Color.red` is in the same file (fixture `proj` lines 43-44, `iso/matchdot`).
- **Explicit generics / call operator / paren-less / value refs / registrar arguments / arity**: none apply or are recorded. `ExtractCtx` is ignored (`_ctx`, `lean.rs:68`), so `--reference-edges`, registrar verbs and `RefRecord::value_ref` produce nothing. `RefRecord::arity` is never set (`push_ref`, `lean.rs:481-495`, sets only name/receiver/site).

## 04. Imports & module resolution

- **`import A.B.C`**: an `ImportRecord{kind:"import", path:"A.B.C"}` (`lean.rs:230, 281-296`). Test `imports_and_opens`.
- **`open Nat List`**: one record per opened namespace (`kind:"open"`), split on whitespace gaps (`lean.rs:179-219, 231`). Test `imports_and_opens`.
- **Resolver use**: in `cross_file`, Lean shares the Haskell/Erlang/Elixir/F#/OCaml/Julia branch (`cross_file.rs:2064-2080`).
  - Each import/open path goes into `unqualified_prefixes`, and its last segment becomes a `module_aliases` key.
  - These feed `module_files` / `path_fragments` for the scoped same-name narrowing (`cross_file.rs:2175-2214`). CHANGELOG.md:1301 lists Lean among the languages where a call now reaches the copy in the module it imports.
- Fixture results (`imp/*`; `A/Lib.lean` defines `A.go` and `B/Lib.lean` defines `B.go`):

  | Fixture | Source | Result |
  | --- | --- | --- |
  | `importopen` | `import A.Lib` + `open A` + `go n` | `A.go` **high**, `cross-file:imports` |
  | `openonly` | `open B` + `go n` (no import) | `B.go` **high** |
  | `importonly` | `import A.Lib` + `go n` (no open) | `A.go` **medium**. Lean itself would reject this: `go` is not in scope without `open A`. |
  | `none` | no import/open, two candidates | unresolved `candidates-in-other-files {candidates: 2}` |
  | `qualA` | `A.go n` | `A.go` high |
  | `p2` `callB` | `B.foo 3` with B never imported | `B.foo` **high**. A qualified path binds whether or not it is imported. |

- **Current namespace not implicitly opened**: inside `namespace B`, a bare `go` does **not** resolve to `B.go` (fixture `imp/ns_same`: unresolved). This is a gap: Lean resolves it.
- **`export A (foo)`, `open A (x y)`, `open A renaming …`, `open A hiding …`, `open … in`**: no dedicated handling (unverified for the selective forms). `open X in def …` loses the def entirely (§02).
- **Aliases**: `ImportRecord.alias` is always empty (`lean.rs:291`).
- **`FileFacts::packages`**: not set by the Lean plugin (no `packages` in `lean.rs`).
- **Manifest reading** (`lakefile.lean`, `lakefile.toml`, `lake-manifest.json`): none. `plugins/manifest.rs` has no Lean entry.
- **External-module refusal (`heads.rs` `WorkspaceVocab`/`FileHeads`)**: not applicable. heads.rs covers only Python/JS/TS/Go/Java/Kotlin/C#/C++, and there is no `lean` in heads.rs.
- **`not-in-imported-module` (`modules.rs`)**: not applicable; there is no `lean` in modules.rs or libscope.rs.
- **External bucket**: a name no workspace callable carries is filed as external by the generic name screen `classify_external` (`crates/cgg-core/src/external.rs:145-168`, `name_screen_applied: "external"`). Fixture: `Nat.add_zero`, `IO.println`, `rfl`, `List`.

## 05. Type inference & receiver typing

- **Parameter typing from `(name : Type)`**: the generic colon-parameter parser types Lean binders. A cons pattern `x :: xs : List α` is explicitly refused (`crates/cgg-resolve/src/type_hints.rs:2021-2024`, test assertion at `type_hints.rs:2996`).

  ```lean
  def useP (p : P) : Nat := p.x   -- binds P.x high (fixture iso/typed)
  def useQ (q : Q) : Nat := q.x   -- binds Q.x high, not P.x
  ```

- **File-wide locals, last write wins (false `high`)**: Lean is not one of the scoped-local languages. In `iso/typed`, `def useU := fun p => p.x` (untyped `p`) bound **`P.x` at `high`**, using the type `p` received in `useP`, although `Q.x` is an equally valid candidate.
- **Applied types are not reduced**: a receiver typed `List Nat` keeps the whole string as its type.
  - `def List.sumIt …` + `(xs : List Nat) … xs.sumIt` is **unresolved** (receiver_hint `List Nat`, fixture `iso/gen` g1, `p2` genField).
  - `(xs : List)` binds high (g2), and an untyped `fun xs => xs.sumIt` binds medium (g3).
  - `declared_type` strips only `?`/`!`/unions/subscripted wrappers (`crates/cgg-core/src/annotation.rs:30-68`), so Lean juxtaposition types (`Option P`, `Array Foo`) are never unwrapped.
- **Literal receivers**: `x.go` with `(x : Nat)` → receiver `Nat`, unresolved (fixture `iso/fan`).
- **Not supported**:
  - Local `let` typing.
  - Return-type propagation via `type_hints::build_return_type_map`: unverified for Lean, with no Lean-specific code.
  - Casts/constructions (`CAST_INTRINSICS`/`CONSTRUCTING_METHODS` have no Lean entries).
  - `FileFacts::scoped_types` (`closure_types.rs` has no Lean).
  - `implicit_receiver` and `weak_receiver_type`.
  - Typeclass instance resolution and generalized field notation via the elaborated type (`lean.rs:22-26`).

## 06. Inheritance, dispatch & overloading

- **Base-class records**: none. `structure P extends Q` records no base, so `p.q` with `(p : P)` is unresolved (fixture `iso/extends`).
- **Typeclass methods**: `Shape.area p` binds to the class field `Shape.area` (a property) at high (fixture `proj`, `iso/classm`). The instance's implementation (`area p := …`) is not a callable, so there is no class → instance edge.
- **Not in `INHERITANCE_LANGS`** (`crates/cgg-resolve/src/dispatch.rs:110-123`), so `inheritance_fanout` does not run for Lean.
- **`--dynamic-dispatch`**: no effect. The plugin never sets `trait_impl_target` (not in `lean.rs`), and the dead-code capability table shows `disp no` (fixture `proj` `--dead-code`).
- **Not in `OVERLOAD_ARITY_LANGS`** (`type_hints.rs:1583` = java/scala/cpp), `binds_same_name_set` (`intra_file.rs:835-850`) or `is_clause_language` (`intra_file.rs:857-859`).
- **Same-name candidates**: three namespaces each defining `go`, with a bare `go` and no open → `ambiguous-in-file`, refused (fixture `iso/fan`). Nothing is fanned out.
- **`super` / `member_of_calling_class` / `overrides_below` / extension methods / mixins**: not applicable. There are no such Lean constructs in the model.

## 07. Scoping & shadowing

- **Binder shadowing**: names bound by the declaration's binders suppress unqualified references in its body (`lean.rs:396-429, 485-487`). Test `a_binder_name_is_the_local_not_a_declaration_elsewhere`. Bindings nest (outer `bound` is restored, `lean.rs:400-403`).
- **`FileFacts::locals`**: not populated (no `locals` in `lean.rs`, no Lean in `cgg-lang/src/locals.rs`).
  - `let f := fun k => …; f n` therefore records `f` as a call. It goes to the external bucket rather than being dropped as `shadowed-by-local` (fixture `proj` line 32).
  - `fun x =>`, `match … with | k+1 =>` and `have h :=` binders are not tracked. Only declaration-signature binders are.
- **Lexical narrowing**: Lean is **not** in `lexically_scoped` (`crates/cgg-resolve/src/intra_file.rs:119-136`). This mostly does not matter, because nested defs (`where`, `let rec`) are not extracted at all.
- **Positional shadowing**: none (only OCaml/F#/Lua have it).
- **Conditional definitions**: not applicable. Lean has no preprocessor, and `DefRecord::conditional` is not set.
- **Namespace / section scoping**: names are prefixed by the namespace stack (`lean.rs:161-177`). `section` scoping of `variable` declarations is not modelled. `variable (m : Nat)` + `def inSec := foo m`: `m` is not a binder of `inSec`, but because bare identifiers are not recorded in term mode no false reference appears (fixture `p2`: `inSec -> A.foo` only).

## 08. Macros, preprocessor & codegen

- **No macro expansion**: `macro`, `macro_rules`, `syntax`, `elab`, `notation`, `infix`/`prefix` declarations yield no callable, and their uses are not expanded. Stated at `lean.rs:22-26`.
  - Fixture: `notation "dbl" x => double x` + `def useNotation := dbl 3` → `dbl` goes to the external bucket and there is no edge to `double` (`proj` line 29).
  - `macro …` produces a parse error (`iso/macro`).
- **No `macro:<name>` tagging, no `name_mentions`** (neither appears in `lean.rs`).
- **`deriving Repr`** generates nothing (fixture `iso/deriving`).
- **Attribute command `attribute [simp] b`**: not captured onto `b` (fixture `iso/attrcmd`: `b` has no attributes). Only the inline `@[…]` preamble is read (`lean.rs:259-279`).
- **Preprocessor / `#if` / templates / quote-unquote**: not applicable.

## 09. FFI & cross-language links

- **`@[extern "c_fn"]` / `@[export lean_fn]`**: captured only as the attribute strings `@[extern]` / `@[export]`, with the arguments dropped (`lean.rs:265-268`). `ffi::link_ffi` has no Lean case (no `lean` in `crates/cgg-resolve/src/ffi.rs`), so no Lean↔C edge is created. The dead-code FFI-export root table is keyed on other attributes (unverified whether `@[export]` matches `FFI_EXPORT_ATTRS`; not tested).
- **Descriptor links (protobuf rpc → host)**: not applicable (no Lean in `descriptor.rs`).
- **Markup code-behind**: not applicable.

## 10. Frameworks & entry points

- **No framework rules apply to Lean.**
  - Per-language counts of `language: "…"` in `crates/cgg-core/src/frameworks/rules.rs` list 36 languages and no `lean`.
  - `grep -n lean rules.rs` matches only `orleans` (`rules.rs:10357`) and string fragments.
  - There are no `member_sources_for`, `signature_methods_for`, `container_attributes_for` or `markup_kinds_for` entries for Lean.
- **Registrar verbs**: not applicable. `ExtractCtx` is unused (`lean.rs:68`).
- **Benchmark coverage**: the only corpus entry is `lean-batteries` (`scripts/benchmark.sh:93`, `scripts/update-readme-stats.sh:94`), with no framework `APPS` for Lean. README benchmark row: Batteries, 2,335 callables, 7,472 edges, 59% (`README.md:1345`).
- **Entry points actually recognised**:
  - `main`, through the universal entry list (`crates/cgg-resolve/src/deadcode/roots.rs:20`). Fixture: `main` → root kind `program-entry`, rule `builtin:main`.
  - A top-level reference with no enclosing callable (anonymous instance, `example`, a body that failed to parse) roots every same-named callable via `toplevel:invocation` (`roots.rs:593-611`). Fixture `proj`: `instance : Shape Demo.Point where …` rooted `Shape` (`top-level-invocation`).
- **Not modelled**: `#eval`/`#print`/`#check` commands, `@[init]`/`initialize`/`builtin_initialize` declarations, Lake `script`/`lean_exe` targets, `@[command_elab]`/`@[tactic]`/`@[macro]` elaborator registrations, `@[simp]`/`@[ext]`/`@[instance]` attribute-driven use, and typeclass instances (see §12).

## 11. Test harness entries

- **No test rules for Lean**: there are no kind-`test` rules, no LSpec/`#guard`/`#test`/`Plausible` handling, and `@[test]` is captured as an attribute string only (fixture `p2`: `aTest` has `['@[test]']` and is not a root).
- **`classify_test_file`**: no Lean file-name rule (`crates/cgg-core/src/testfile.rs:75-118`, falls to `_ => false`). The generic directory rule applies: a file under `test`/`tests`/`Test`/`Tests`/`spec`/… (`testfile.rs:36-45, 125-130`) gets a test role (`crates/cgg/src/lib.rs:569`). Unverified for Lean by fixture.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: not applicable (Python/PHP/Ruby only).
- **Test-file gating of name-only rules**: not applicable (there are no rules).

## 12. Dead-code analysis

- **Capability class: `degraded`**. Fixture `proj` `--dead-code` shows `lean … class degraded, vis no, attrs yes, refs no, disp no`.
  - `signals()` declares nothing (`lib.rs:304-306`).
  - Attributes are upgraded to "yes" by observation, because some node carried `@[simp]`/`instance` (`crates/cgg-resolve/src/deadcode/caps.rs:130-136, 165-170`).
  - Visibility is never observed (next bullet), so Lean cannot reach `Analyzable` / `signals_complete` (`caps.rs:88-96`). Every finding carries `language-lacks-visibility`, `language-lacks-value-references` and `language-lacks-dispatch-model`, and none reaches `high`.
- **Visibility capture is effectively dead code**:
  - `visibility()` scans the first 48 bytes of the *definition node* for `private`/`protected` (`lean.rs:321-341`).
  - The grammar puts the modifier outside that node: `private def helper` starts at the `def`, byte 61 in fixture `proj`, after `private` at 53-60.
  - As a result `private def helper` and `protected def prot` carry no visibility (fixture JSON: no `visibility` key; dead-code `vis no`).
  - `DefRecord::vis` (the normalized enum) is never set either (`lean.rs:306-318`), so it stays `Vis::Unknown`, which counts as public/escaping (`crates/cgg-core/src/facts.rs:133-136`).
- **`@[simp]` and `instance` markers do not root anything**:
  - The plugin docs and CHANGELOG say they are captured "so dead-code reporting can treat implicitly used declarations as live" (`lean.rs:14-20`, CHANGELOG.md:1840-1842).
  - No consumer reads them: there is no `simp`/`instance` match in `crates/cgg-resolve/src/deadcode/` or `crates/cgg-core/src/frameworks/`.
  - Fixture `proj` reports `Demo.double_eq` (`@[simp]`) and `Demo.instAddPoint` (`instance`) as unreferenced at medium. Only a `--suppress-attribute`-style user filter (`cli --help` line ~312) could exclude them (unverified).
- **Root kinds seen**: `program-entry` (`main`) and `top-level-invocation` (§10). User roots from `cgg-deadcode.toml` work generically.
- **Theorems are reported as dead**: every `theorem`/`lemma` that nothing references by name is a D001 finding, which is the normal state of a library's public theorems (fixture `proj`: `t`, `t2`). Properties and constructors get `implicitly-invokable-kind` (`crates/cgg-resolve/src/deadcode/evidence.rs:234-239`), which lowers them (`Demo.Color.red` low).
- **Unresolved-site doubt**: generic name-match. `Color.red` is lowered to `low` because of the unresolved `.red` site (`name-matches-unresolved-site`, fixture `proj`).
- **`implements-external-contract` / `macro-generated` / `reflection` caps**: never produced for Lean (no override, `macro:` or dynuse signals).
- **`BLIND_SPOTS`**: no Lean entry (`caps.rs:42-67`), so the report names none of Lean's known blind spots (macros, notation, typeclass dispatch).

## 13. Builtins & stdlib tables

- **No `stdlib/lean.txt`**: `crates/cgg-core/src/stdlib/` has 30 tables and none for Lean. `stdlib_names("lean")` returns `None` (`crates/cgg-core/src/stdlib.rs:221-260`).
- **Consequence**: `--include-stdlib` has nothing to classify for Lean. `Nat.add_zero`, `List.map`, `IO.println`, `rfl`, `Inhabited`, `omega` and the like all land in the generic **external** bucket (fixture `proj` audit `external_calls`, run summary `0 stdlib, 18 external`).
- **No `rust_types`-like wrapper tables, no `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` / `MAYBE_TYPE_WRAPPERS` / `UNION_TYPE_CONSTRUCTORS` / `NULL_TYPE_MEMBERS` entries, and no implicit packages** (no `lean` in `stdlib.rs` or `annotation.rs`). `Option`/`Except` are not stripped as maybe-wrappers.
- **Tactic keywords**: the tactic name itself is excluded structurally (the first identifier of `tactic_apply`, `lean.rs:557-562`), plus the 6 modifiers in `is_tactic_modifier` (`lean.rs:100-105`). Bare tactics such as `omega`/`rfl`/`decide` record nothing when they are not inside an application. `rfl` as a term-mode proof is recorded only when it is an operand.

## 14. Confidence & certainty

- **Paths that yield `high`**:
  - intra-file single candidate: `helper -> double`, mutual recursion, `p.x` on a typed param.
  - `cross-file:imports` with an `open` of the namespace, or a fully qualified projection: `useIt -> Demo.double`, `callB -> B.foo`, `c -> A.go`.
  - All fixtures `proj`, `p2`, `imp/*`.
- **False / over-confident `high`**:
  - An untyped `fun p => p.x` bound `P.x` at high through file-wide parameter typing (fixture `iso/typed`).
  - Type-annotation "calls" `useIt -> Demo.Point` and `instSh -> Sh` are emitted at high (fixtures `proj`, `iso/classm`).
  - A qualified `B.foo` with no `import` of B binds high (fixture `p2`). It is structurally right, but nothing checks that the module is imported.
- **`medium`**:
  - Import without open: `imp/importonly`.
  - Untyped receivers: `instAddPoint -> Point.x` from `a.x` with untyped `a`, and `g3 -> List.sumIt`.
  - The CHANGELOG known limitation: dot-projections and names bound inside tactic proofs are resolved by name and are `medium` guesses (CHANGELOG.md:1964-1967).
- **`low`**: only via `--guess-unique-names`, which is Rust-only, so no low edges are produced for Lean (unverified by fixture).
- **Mermaid `?` marker**: generic. An arrow with no high site gets a leading `?` (CLAUDE.md; not Lean-specific).

## 15. Dynamic use & reflection

- **Not applicable / none**: `dynuse` covers Python/JS/Java/C#/Ruby/PHP only (CLAUDE.md §7, with no `lean` in `plugins/dynuse.rs`). The `dyn_uses` signal is false.
- Lean metaprogramming that looks up declarations by name at runtime or elaboration time is not recorded:
  - `evalConst`, `Lean.Environment.find?`, ``Name.mkStr``, `` `foo `` quoted names, `decl_name%`, `#eval`-time `run_cmd`.
  - Quoted names (`` `(foo 0) `` in `macro_rules`) do not reach `foo` (fixture `p2`: `macro_rules |`(mysyn) => `(foo 0)` produced no edge).
- There is no `reflection` or `module-load` screen for Lean.

## 16. Known gaps & limitations

The items below were found in source comments, the CHANGELOG, or fixtures run for this document.

- **Typeclass-method dispatch** (`a + b` → `HAdd` instance), **macro/notation expansion** and **type-directed dot resolution** are explicitly out of scope (`lean.rs:22-26`, CHANGELOG.md:1964-1967).
- **Visibility never captured**: `private`/`protected` sit outside the `definition` node and the 48-byte scan never sees them. `DefRecord::vis` is never set. The dead-code class is stuck at `degraded` (§12, fixture `proj`).
- **`@[simp]`/`instance` markers are recorded but unused** by dead-code or frameworks, contrary to the plugin docs and CHANGELOG (§12).
- **Callables not extracted**: `where` auxiliaries, `let rec`, instance-method bodies, anonymous instances, `instance (priority := …)`, `open … in def`, `macro`/`syntax`/`notation`/`elab` (§02).
- **Call forms silently dropped** (no edge and no audit record): `<|`, `|>`, `$`, `@f`, a function passed as a value, `#eval` bodies (§03).
- **False call references from type annotations** (`List`, `Add`, `IO`, and in-workspace `Demo.Point` / `Sh` edges at high) and from intermediate path segments (`Foo.Bar.baz` → `Bar`) (§03).
- **Applied receiver types** (`List Nat`) break typed method lookup. File-wide parameter typing gives a false `high` to untyped lambdas (§05).
- **No base records for `extends`**, and the current namespace is not implicitly opened (§04, §06).
- **Leading-dot constructors `.red` are never bound** (§03).
- **No stdlib table**, so every Lean core/Mathlib call is reported external rather than stdlib (§13).
- **No framework/test rules**: LSpec, `#guard`, Lake targets, `initialize`, `@[command_elab]`/`@[tactic]` (§10, §11).
- **No `PluginSignals` declared** (`lib.rs:304-306`) and **no `BLIND_SPOTS` entry** (`caps.rs:42-67`).
- **No integration tests or fixtures**: Lean coverage is the 13 inline unit tests in `lean.rs:575-909`. There are no `.lean` files under `crates/*/tests`, and no ignored tests.
- **Corpus coverage**: one repository only (`lean-batteries`, `scripts/benchmark.sh:93`). Mathlib-style `lemma` is handled (`lean.rs:344`), but no Mathlib repo is in the corpus.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.lean` only, vendored grammar; no shebang/content detection; `where`/priority/`macro` produce parse errors |
| 02. Callables extracted | partial | defs/theorems/axioms/structures/inductives/named instances; no `where`/`let rec`/lambdas/anonymous instances/`open … in` defs |
| 03. Call-site forms | partial | application, projection, tactic lemma lists; `<\|`/`\|>`/`$`/`@f`/value refs dropped silently; type annotations recorded as calls |
| 04. Imports & module resolution | partial | `import`/`open` feed unqualified prefixes; no manifest/packages/heads/modules; current namespace not auto-opened |
| 05. Type inference & receiver typing | minimal | `(x : T)` binders typed file-wide; applied types (`List Nat`) unusable; false high from file-wide typing |
| 06. Inheritance, dispatch & overloading | none | no `extends` bases, no instance dispatch, not in any dispatch/overload list |
| 07. Scoping & shadowing | minimal | declaration binders only; no `FileFacts::locals`, not lexically scoped |
| 08. Macros, preprocessor & codegen | none | no macro/notation/deriving expansion; `attribute [..]` command ignored |
| 09. FFI & cross-language links | none | `@[extern]`/`@[export]` kept as attribute strings only |
| 10. Frameworks & entry points | minimal | no rules; only universal `main` and top-level-invocation roots |
| 11. Test harness entries | none | no test rules; generic test-directory classification only |
| 12. Dead-code analysis | minimal | class degraded; visibility never captured; `@[simp]`/`instance` markers unused; theorems reported dead |
| 13. Builtins & stdlib tables | none | no `stdlib/lean.txt`; every core call is external |
| 14. Confidence & certainty | partial | high via intra-file/open/qualified; false highs from file-wide typing and type-annotation edges |
| 15. Dynamic use & reflection | none | no dynuse; quoted names/`evalConst` not modelled |
| 16. Known gaps & limitations | n/a | documented above; only inline unit tests, one corpus repo |
