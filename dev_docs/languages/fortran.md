# fortran

The Fortran plugin (`FortranPlugin`, `crates/cgg-lang/src/plugins/fortran.rs`, 237 lines including one inline test) parses Fortran with the crates.io grammar `tree-sitter-fortran = "0.6"` (`Cargo.toml:111`, `crates/cgg-lang/Cargo.toml:49`, bound at `fortran.rs:21-23` via `tree_sitter_fortran::LANGUAGE`). Nothing is vendored. One recursive `FortranWalker` (`fortran.rs:44-203`) handles five node kinds and nothing else:

- `module_statement`: sets a module-name prefix.
- `use_statement`: becomes a `use` import.
- `function` / `subroutine` / `program`: each becomes a `FreeFunction` definition.
- `call_expression` / `subroutine_call`: each becomes a bare-name reference.

`extract` ignores `ExtractCtx` (`_ctx`, `fortran.rs:27`), and the plugin keeps the default `signals()` (all false, `crates/cgg-lang/src/lib.rs:304-306`). There is no separate resolver kind on the trait. Fortran goes through the generic `intra_file` → `cross_file` pipeline and has no Fortran-specific branch apart from one `use` comment (`cross_file.rs:1941`) and an `include` arm that the plugin never feeds (`cross_file.rs:2130`). It is one of the thinnest plugins in the tree.

## 01. Detection & parsing

Supported:

- **Extensions** `.f90 .f95 .f03 .f08 .f .for .fpp` (`fortran.rs:15-17`). The detector lowercases the extension on a second pass (`crates/cgg-lang/src/detect.rs:116-123`), so `.F90` and `.F` are detected too.
  - Fixture `a/pre.F90`: `detected_via: extension:.f90`.
  - Fixture `g/up.F`: `extension:.f`.
- **Shebangs**: none (`fortran.rs:18-20`). That is normal for a compiled language.
- **Grammar**: one grammar for every extension. There is no `ts_language_for` override, so fixed-form and free-form source share the free-form grammar.
- **Depth guard**: the generic `MAX_TREE_DEPTH = 4000` / `SkipReason::TooDeep` applies (`crates/cgg/src/lib.rs:155`, `:451-454`). Nothing Fortran-specific.
- **Long lines**: `max_line_bytes` is not overridden (default `None`, `lib.rs:313-315`), so no long-line skip applies.

Not supported / partial:

- **`.f77`, `.ftn`, `.f18`, `.F77`**: not in the extension list. Fixture `g/old.f77` → `file_skipped … unknown-extension`.
- **`.inc` / `.h` Fortran include files**: never analyzed. Fixture `a/defs.inc` → `unknown-extension`.
- **Fixed-form source** (`.f`, `.for`, column-1 `C`/`*` comments, statement labels) parses with errors because the grammar is free-form.
  - Fixture `g/fixed.f` → `parse_status: error`. Error recovery still produced `LEGACY`, `STEP` and `FN` and the edge `LEGACY -> STEP`.
  - The function reference `X = FN(I)` inside `STEP` produced no edge and no audit record. The site was silently lost.
  - The fixed-form `util.f` without comment lines parsed `ok` (fixture `a/util.f`).
- **No re-parse or fallback.** Unlike C#'s `#if` blanking, a file with parse errors is extracted from the error-recovered tree as-is.
- **Preprocessed `.F90` / `.fpp`**: `#ifdef`/`#else` lines are passed to the grammar raw (see §08).
- **Minified-file skip**: not applicable (that rule lives in `locals.rs`, which Fortran does not use).

## 02. Callables extracted

Every callable is `DefVariant::FreeFunction` with empty visibility and no attributes (`fortran.rs:156-173`). The qualified name is `<module>::<name>` when a `module_statement` has been seen earlier in the file, otherwise `<name>` (`fortran.rs:54-60`). The span covers the whole enclosing form (`fortran.rs:72-75`), and `end_line` is the start of the next line (fixture: `geom::area 24-29`, `geom::shape_area 29-33`). `signature_hint` comes from the generic `extract_signature` (`fortran.rs:169`).

Supported:

- `subroutine s(x) … end subroutine` → `s` (fixture `a/shapes.f90`: `geom::helper`).
- `function f(x) result(r)`, `real function f(x)`, and prefixed `pure` / `elemental` / `recursive` forms → `f` (`geom::area`, `geom::shape_area`, `geom::rec`).
- `program p … end program` → `p`, same variant as a subroutine (`driver`, `caller`).
- Module procedures after `contains` → `mod::name` (`geom::make_real`, …).
- Internal procedures (`contains` inside a subroutine) → extracted. The qn is flattened to the module, not the host: `subroutine rec … contains … subroutine inner` → `geom::inner`, not `geom::rec::inner` (fixture `a`).
- Fixed-form `SUBROUTINE UTIL_SUB(X)` → `UTIL_SUB`, case preserved as written (fixture `a/util.f`).
- Interface bodies (explicit `interface`, `abstract interface`, `module subroutine` prototypes in a parent module) are extracted as ordinary callables:
  - fixture `geom::cb_t` from `abstract interface`;
  - `parent_m::sm_proc` twice: once for the interface body, once for the submodule body;
  - `fmod::c_side` for a `bind(c)` interface (fixture `f`).

  They carry no `abstract` tag, so they compete with real bodies (see §06, §09).

Not supported / partial:

- **The module prefix leaks past `end module`.** `self.module` is never reset (`fortran.rs:99-110` sets it, and nothing clears it).
  - Fixture `b/m1.f90`: top-level `subroutine after_mod` after `end module m1` → `m1::after_mod`.
  - Fixture `f`: `program main` after `end module fmod` → `fmod::main`.
  - Fixture `g/nest.f90`: `program pr` after `module mb` → `mb::pr`.
- **Submodules**: `submodule (parent_m) child_s` is not recognised. Its procedures inherit whatever module was last set (`parent_m::sm_helper` in fixture `a/sub.f90`), and the submodule name is never used.
- **Separate module procedures** `module procedure foo_mp … end procedure`: not extracted (fixture `a/sub.f90`: no `foo_mp`).
- **`entry alt_entry(x)`**: an alternate entry point, not extracted. `call alt_entry(1.0)` → external, `no-candidate-in-file` (fixture `b/caller.f90:24`).
- **Statement functions** `sq(x) = x*x`: not extracted. Both the definition line and the use are recorded as calls to `sq` → external (fixture `b/caller.f90:16-17`).
- **`block data`**: not extracted (fixture `a/sub.f90`).
- **Derived types** (`type :: shape_t`): no class or owner record. Type-bound procedures (`procedure :: get_area => shape_area`) are not linked to their binding name or owner. The implementing function is a plain module function (`geom::shape_area`).
- **Generic interfaces** (`interface make; module procedure make_real, make_int`) and **operator interfaces** (`interface operator(+)`): no generic node is created. The specifics appear only as their own module functions.
- **Procedure pointers** and `procedure(cb_t) :: f` dummy procedures: no record.
- **Variant**: PROGRAM, SUBROUTINE and FUNCTION are indistinguishable. This is acknowledged in the `fortran-program` rule's `gap` text (`crates/cgg-core/src/frameworks/rules.rs:6763`).

## 03. Call-site forms

`record_call` (`fortran.rs:176-202`) takes the first direct `identifier` child of a `call_expression` or `subroutine_call` and pushes a `RefRecord` with an empty `receiver_hint` and no other field set.

Supported:

- `call s(x)` → bare `s` (fixture: `UTIL_SUB -> HELPER2`, `rec -> inner`).
- A function reference in an expression, `y = f(x)` → bare `f` (fixture: `shape_area -> area`, `make_int -> make_real`).
- A recursive self call `call rec(n-1)` → a self edge `geom::rec -> geom::rec` (fixture `a`, intra-file high).
- Calls nested in arguments, e.g. `make_real(real(x))`: both `make_real` and `real` are recorded (`real` → stdlib bucket).

Not supported / partial:

- **Type-bound and component calls** `s%get_area()` / `call s%helper()`: the callee child is a `derived_type_member_expression`, not an `identifier`, so nothing is recorded. There is no edge, no unresolved entry and no audit record (fixture `a/main.f90:9-10`; absent from `external_calls`).
- **Array element references look like calls.** `v = arr(i)` is recorded as a call to `arr` → external bucket (fixture `a/main.f90`, `b`). Locals are not modelled, so nothing filters it out.
- **Dummy procedure calls** `f(x)` with `procedure(cb_t) :: f` → external `f` (fixture `a/shapes.f90:71`).
- **Procedures passed as actual arguments** `call apply(area_cb, x)`: `area_cb` is not recorded as a value reference, so `area_cb` reads as never-referenced (fixture `a`, dead-code finding `geom::area_cb`).
- **Generic-name calls** `make(2.0)`: recorded as `make`, but no callable carries that name → external (fixture `a/main.f90:7`).
- **Operator use** `a + b` against `interface operator(+)`: not a call.
- **Renamed use** `use mb, only: ar => area; call ar()` → `ar` external (fixture `g/nest.f90:23`).
- **No receiver, arity, `value_ref`, `typed_receiver` or `path_receiver`** fields are ever set (`fortran.rs:188-194`). There is no explicit-generics form in the language.
- **No registrar-argument capture** (no `ctx` use, no `registrar.rs` call).
- **I/O statements** (`print`, `write`, `read`, `open`) are statements, not call nodes. They are listed in the stdlib table (§13).

## 04. Imports & module resolution

Supported:

- **`use module_name`** → `ImportRecord { kind: "use", path: name, alias: "" }` (`fortran.rs:112-130`).
- **Generic `use` arm**: `cross_file.rs:1940-1955` pushes the path onto `unqualified_prefixes`.
- **Step 1b** (`cross_file.rs:5944-5970`) then looks up `<prefix>::<name>`. It binds at high when exactly one opened module declares the name.
  - Fixture `b/caller.f90`: `use m2` + `call work()` → `caller -> m2::work` high `cross-file:imports`, with `m1::work` and `m3::work` existing too.
  - Fixture `a`: `driver -> geom::area` high.
- **`use m, only: a, b`**: `only:` lists are not parsed (only the module name child is read, `fortran.rs:115-118`). The import works as a whole-module open.
- **Step 1d by-simple fallback** (`cross_file.rs:6014-6055`). When the file has any import and ≤3 callables carry the name, it binds every candidate at medium. This applies even to names the used module does not declare.
  - Fixture `b`: `call only_m1()` with only `use m2` → `caller -> m1::only_m1` medium.
  - Fixture `e`: three copies of `shared` + `use mm` → three medium edges. Four copies → refused.

Not supported / partial:

- **Files with no `use` cannot reach any other file.** Step 1d requires `has_imports`. A plain F77-style `call uniq()` with exactly one `subroutine uniq` elsewhere stays unresolved (`candidates-in-other-files`).
  - Fixture `d`: `0 edges, 2 unresolved`.
  - Fixture `c`: two copies → `candidates-in-other-files {candidates: 2}`.

  Most legacy Fortran has no modules, so this is the largest resolution gap.
- **`use` scope is file-wide, not per program unit.** Fixture `b/caller.f90`: `subroutine nouse` (no `use` of its own, after `program caller` which has `use m2`) → `nouse -> m2::work` **high**.
- **Rename lists** `use m, only: local => remote` and `use m, local => remote`: the alias is ignored (`alias: String::new()`, `fortran.rs:123`). A call through the local name is unresolved/external (fixture `g`).
- **`include 'file.inc'` and `#include "file"`**: never recorded as imports.
  - The plugin only matches `use_statement` (`fortran.rs:68`).
  - The `include` arm in `cross_file.rs:2124-2150`, which tries `{path}.f90`/`.f95`, is unreachable for Fortran.
  - Fixture `a/inc.f90`: `call from_inc()` → external `no-candidate-in-file`.
- **Intrinsic modules** (`iso_c_binding`, `iso_fortran_env`, `ieee_arithmetic`, `omp_lib`, `mpi`): become unqualified prefixes that match nothing. They are not classified as stdlib/external modules, and `heads.rs` has no Fortran arm (grep: no `fortran` in `heads.rs`, `modules.rs`, `libscope.rs`).
- **`FileFacts::packages`**: not populated. There is no manifest reading (fpm `fpm.toml`, CMake), and `manifest.rs` has no Fortran use.
- **External-module refusal** (`heads.rs` `WorkspaceVocab`) and **`not-in-imported-module`** (`modules.rs`): Fortran is not among the languages, so neither applies.
- **Submodule parent link** `submodule (parent)`: not an import.

## 05. Type inference & receiver typing

Not applicable for practical purposes: no type facts are emitted.

- The plugin sets no `local_types`, `scoped_types`, `class_fields`, `arg_type`, `typed_receiver` or `weak_receiver_type` (`fortran.rs:188-194`, and grep finds no `fortran` in `type_hints.rs` or `receiver.rs`).
- Declarations like `type(shape_t) :: s` and `class(shape_t), intent(in) :: self` are ignored. `s%get_area()` is not even recorded (§03), so no receiver typing could apply.
- `declared_type`, nullability wrappers, closure-parameter typing (`closure_types.rs`) and implicit receivers: not applicable (no closures, no receivers recorded).
- `return-type` maps: function `result(...)` types are not read.

## 06. Inheritance, dispatch & overloading

None:

- **Type extension** `type, extends(base) :: child`: no `ClassDecl` or bases are recorded. `resolve_via_bases`, `member_of_calling_class`, `overrides_below` and `super` handling therefore never apply.
- **Not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS`** (grep finds no `fortran` in `dispatch.rs` or `type_hints.rs`). `--dynamic-dispatch` and `inheritance_fanout` produce nothing. `trait_impl_target` is not set (`signals().impls = false`).
- **Deferred bindings / abstract types** (`procedure(iface), deferred :: m`): not modelled.
- **Generic interfaces as overload sets**: not modelled (§02). `make(2.0)` is not narrowed to `make_real`/`make_int`.
- **Interface bodies vs implementations**: the interface body and the body of the same name are both plain callables with no `abstract` tag, so the "drop abstract when a concrete candidate survives" narrowing cannot apply. Fixture `f`: `call c_side(x)` binds to the interface prototype `fmod::c_side` (intra-file high), not to the C definition.
- **Same-name internal procedures in different hosts** (both qualified `ma::helper`) are not told apart. Each host's call binds to both at medium through cross-file (fixture `g/nest.f90`: `host1 -> ma::helper` ×2, `host2 -> ma::helper` ×2), so each host gets one wrong-target edge.
- Extension methods and mixins: not applicable to the language.

## 07. Scoping & shadowing

None / minimal:

- **No `FileFacts::locals`**: no `fortran` entry in `crates/cgg-lang/src/locals.rs`. `shadowed-by-local` never fires, and an array or local variable indexed like a call is reported as an external call (§03).
- **No lexical narrowing of nested definitions.** Fortran is not in the narrowing list in CLAUDE.md, and fixture `g/nest.f90` shows host-contained procedures are not preferred.
- **No positional shadowing** and no `DefRecord::conditional`.
- **Intra-file linking is name-exact and case-sensitive**, even though Fortran is case-insensitive.
  - Fixture `h/a.f90`: `subroutine work` + `call WORK()` in the same file → `WORK` external.
  - Fixture `b`: `call WORK()` against `m2::work` → external.
  - Intra-file binds only when one same-file callable carries the exact name (fixtures `a`/`b`: unique names → intra-file high).
- **The module-prefix leak** (§02) also leaks scope: in fixture `g`, `program pr` (`mb::pr`) calling `area()` binds intra-file high to `mb::area` even though its `use mb, only: ar => area` makes only `ar` visible.

## 08. Macros, preprocessor & codegen

None:

- **`#ifdef` / `#else` / `#endif`** in `.F90`/`.fpp`: not interpreted. Both branches are extracted as duplicate callables, and both bodies' calls bind. Fixture `a/pre.F90`: `par_a` lines 3-6 and 7-10, each `-> helper3` high.
- **`#include` / `#define`**: not recorded (no `c_macro.rs` use; `fn_macros` and `macro_invocations` stay empty).
- **No `macro:<name>` tagging** and no `name_mentions`.
- **Fypp / template preprocessors** (used by fortran-lang/stdlib's `.fypp` files): `.fypp` is not an extension, so those files are skipped entirely (unverified on the corpus; follows from the extension list `fortran.rs:16`).
- Fortran has no quote/unquote or templates (Fortran 2023 templates are not supported by the grammar; unverified).

## 09. FFI & cross-language links

None:

- `ffi.rs` has no Fortran handling (grep finds no `fortran` or `bind(c` in `crates/cgg-resolve/src/ffi.rs`).
- **`bind(c, name="f_side")` procedures are not linked to C callers.** Fixture `f`: C `c_side` calls `f_side`, and the edge goes to C's own prototype `void f_side(double)` in `c.c` (intra-file high), not to `fmod::f_side`.
- **Fortran calls into C** via a `bind(c)` interface bind to the interface body (fixture `f`: `fmod::f_side -> fmod::c_side`, the Fortran prototype at lines 6-10), not to the C definition `c_side` in `c.c`. `ffi_detected: 0`.
- The `bind(c)` attribute is not read at all (`attributes: Vec::new()`, `fortran.rs:171`), so it is neither an FFI export root (§12) nor an FFI link.
- f2py / Cython / ctypes (Python → Fortran): not modelled.
- Descriptor links and markup code-behind: not applicable.

## 10. Frameworks & entry points

Rules that apply to `fortran` (grep `language: "fortran"` in `crates/cgg-core/src/frameworks/rules.rs`, one hit):

- **`fortran-program`** (`rules.rs:6736-6766`), kind `TrustKind::Cli`, `node: true`.
  - **Trigger**: detect-only. Its `detect` list holds the import prefixes `iso_fortran_env`, `ISO_FORTRAN_ENV`, `iso_c_binding`, `ISO_C_BINDING`, `ieee_arithmetic`, `IEEE_ARITHMETIC`, `omp_lib`, `OMP_LIB`, `mpi`, `MPI`, `mpi_f08`, `netcdf`, `hdf5`.
  - **Empty fields**: `detect_paths`, `detect_calls`, `attributes`, `registrars`, `base_types`, `methods` and `observer_types` are all `NONE`.
  - **Effect**: when a file `use`s one of the prefixes, `--framework-coverage`/stderr lists it under "seen, no rules … entries NOT enumerated" and prints the `gap` text. That text explains that PROGRAM units, `bind(c)` procedures and callback actuals are the real entry points and cgg cannot see them. Fixture `f`: `seen, no rules fortran-program — found in 1 file(s)`.
  - **No `<framework-entry>` node is ever minted.**
  - Benchmark corpus: `app-fpm-fortran|…|~fortran-program` (`scripts/benchmark.sh:323`). The `~` marks an expected detect-only rule.
  - Test: `crates/cgg/tests/detect_prefixes.rs:86` writes `program p / use <prefix>` to prove every `detect` prefix fires.
- **Multi-language rules naming fortran**: none found.
- **Also absent**: `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for` and visibility entries (no Fortran rows), and registrar verbs (`ExtractCtx` is unused).
- **History**: Fortran used to be the "language with no rules" in the tests and now is not (`crates/cgg/tests/frameworks.rs:441-445`, `crates/cgg-resolve/src/frameworks/mod.rs:2542-2545`, which switched to verilog).
- **Not supported**: OpenMP/OpenACC directives, MPI callbacks, `fpm` `[[executable]]` / `[[test]]` manifest entries, CMake `add_executable` targets.

## 11. Test harness entries

None:

- No rule of kind `test` exists for Fortran. pFUnit (`@test` annotations in `.pf` files), test-drive (`new_unittest("name", proc)` registration), veggies/garden, FRUIT and Fortuno are all unrecognised (grep `rules.rs`: only `fortran-program`).
- `.pf` pFUnit files are not even detected (not in the extension list).
- `signals().test_defs = false`. There is no test-file gating and no `TEST_CLASS_MEMBERS_ARE_ROOTS` entry (classes are not modelled).

## 12. Dead-code analysis

Minimal:

- **Capability row** (fixture `a`, `--dead-code --dead-code-format json`): `class: degraded`, `visibility/attributes/value_references/dispatch/exports/test_tagging: none`, `max_confidence: medium`, `root_rules_active: []`. Degraded is assigned because neither visibility nor attributes are signalled (`crates/cgg-resolve/src/deadcode/caps.rs:165-172`). Fortran is not in `SCRIPT_DRIVEN` (`caps.rs:27-36`).
- **Findings are always medium and so withheld at the default high threshold.** Fixture `a`: `0 shown (high) · 14 withheld (medium)`. Each carries the evidence `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges`, `no-unresolved-site-with-name`.
- **Roots**:
  - Fortran is not in `LANG_ENTRY` (`crates/cgg-resolve/src/deadcode/roots.rs:23-30`). Only `UNIVERSAL_ENTRY = ["main"]` (`roots.rs:20`, `:741-756`) applies.
  - A `program driver` is **not** a root. Fixture `a`: `roots 0 discovered · root-reachability disabled:no-roots-found`, so `reachable-only-from-dead-code` findings (9) are withheld.
  - Only a program literally named `main` roots, as `builtin:main` (fixture `f`: `roots 1 discovered`).
- **`bind(c)` procedures are not FFI-export roots**: attributes are empty, so `classify_ffi`/`FFI_EXPORT_ATTRS` (`roots.rs:760-777`) cannot see them. Fixture `a`: `geom::c_entry` is reported never-referenced.
- **`public`/`private` statements and attributes are not read**, so there are no `ExportedApi` roots (`exports` empty). Fixture `a`: module `public :: area` does not root `geom::area`.
- **No reflection, macro-generated or implements-external-contract caps** (no signals feed them).
- **Unresolved-doubt handling**: generic only. A finding with an unresolved same-name site would be marked, but case mismatches (`WORK` vs `work`) are not matched as the same name (§07).
- **False-dead sources**: procedures passed as actual arguments (`area_cb`), type-bound implementations reached only through `obj%binding` (`shape_area`), generic-interface specifics (`make_int`, `add_shapes`), and `UTIL_SUB` called as `util_sub` from another file (fixture `a`, all in the findings list).

## 13. Builtins & stdlib tables

Partial:

- `crates/cgg-core/src/stdlib/fortran.txt`: 163 entries, all lowercase.
  - Intrinsic procedures: `abs`, `achar`, `allocated`, `sqrt`, `real`, …
  - ISO_C_BINDING procedures: `c_loc`, `c_f_pointer`, `c_associated`, `c_f_procpointer`, …
  - Statement keywords: `allocate`, `backspace`, `open`, `print`, `read`, `write`.
- The table is registered at `crates/cgg-core/src/stdlib.rs:210`, `:251`. CHANGELOG (`CHANGELOG.md:5975`) records it as "seeded from language references only", never tuned against corpus noise.
- **Screening** (`crates/cgg-core/src/external.rs` rule (c), ~line 255): a bare call whose name is in the table, with no project callable of that name, goes to the stdlib bucket (fixture `c`: `sqrt` → `stdlib_calls`; fixture `a`: `real`). `--include-stdlib` mints `<stdlib>::sqrt` with a `std` label (fixture `c` mermaid: `N2 -->|std| N4`).
- **Every other unmatched bare name goes to the external bucket**, not "unresolved". That covers array references, dummy procedures, unknown library routines and case mismatches (`external.rs` "Rule 1" ~line 296). `--include-external` mints `<external>::ext_thing` (fixture `c`).
- **Case-sensitive matching**: `SQRT(2.0)` and `Abs(y)` are classified external, not stdlib (fixture `h`). That misfiles every uppercase intrinsic in fixed-form code.
- **No MPI / OpenMP / BLAS / LAPACK names** in the table (grep `^mpi`, `^omp` → none). `call mpi_init(ierr)` would be external.
- `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` / implicit packages / `rust_types`-style wrapper tables: no Fortran entries (not applicable; no receivers are recorded).

## 14. Confidence & certainty

- **high**:
  - Intra-file binding to a unique exact-case same-file name (fixtures: `shape_area -> area`, `UTIL_SUB -> HELPER2`, `par_a -> helper3` ×2 even across `#ifdef` branches, `rec -> rec`).
  - Step 1b `use`-prefix lookup when exactly one opened module declares the name (`cross_file.rs:5953-5967`; fixture `caller -> m2::work`, `driver -> geom::area`).
- **medium**: the step 1d by-simple fallback (≤3 candidates, file has any `use`; fixture `caller -> m1::only_m1`, the three `shared` edges, the `ma::helper` cross-host edges).
- **low**: never. `--guess-unique-names` is Rust-only per CLAUDE.md.
- **Mermaid `?` marker**: shown on medium arrows (fixture `b` mermaid: `N0 -->|?| N12`).
- **False highs** come from file-wide `use` scope (`nouse -> m2::work`), the module-prefix leak (`mb::pr -> mb::area` despite a renaming `only:`), and interface prototypes standing in for C or submodule bodies (`fmod::f_side -> fmod::c_side`).

## 15. Dynamic use & reflection

Not applicable:

- Fortran has no reflection or eval.
- `dynuse.rs` has no Fortran coverage (`signals().dyn_uses = false`; grep finds no `fortran` in `crates/cgg-lang/src/plugins/dynuse.rs`).
- The closest dynamic forms, procedure pointers (`p => f; call p()`) and `dlopen`/`c_funloc` lookups by string, are not modelled. `call p()` is recorded as bare `p` → external.

## 16. Known gaps & limitations

- **The rule `gap` text** (`rules.rs:6763`) itself states the main ones:
  - PROGRAM, SUBROUTINE and FUNCTION are recorded identically;
  - `bind(c)` is not read;
  - `public`/`private` is not read;
  - callback actuals are not referenced.
- **README** language table: `| Fortran | use module | — | Subroutines and functions |` (`README.md:512`). Benchmark: `stdlib | fortran | 335 | 181 | 4%` cross-file (`README.md:1338`).
- **Case-insensitivity is not modelled anywhere** (intra-file, cross-file, stdlib). Fixtures `b`, `h`.
- **No cross-file resolution without a `use`**, which covers all F77-style code. Fixtures `c`, `d`.
- **The module prefix leaks** past `end module`, so later top-level units are misqualified and bind intra-file to that module's procedures. Fixtures `b`, `f`, `g`.
- **Type-bound / `%` calls are silently dropped**, with no audit record (fixture `a`). This invisibly loses every OO Fortran edge.
- **Array references read as external calls**; there is no locals model (fixture `a`).
- Further gaps, covered in their sections:
  - not extracted: submodules, separate module procedures, `entry`, statement functions, `block data`, generic/operator interfaces (§02);
  - interface bodies duplicate real callables (§02, §06);
  - `use` renames and `only:` lists are ignored; `include` is not followed (§04);
  - fixed-form parses with errors and loses sites; no `.f77`/`.inc`/`.pf`/`.fypp` (§01);
  - preprocessor branches are both extracted (§08);
  - no FFI linking for `bind(c)` (§09);
  - PROGRAM units are not dead-code roots (§12).
- **Test coverage is minimal.** There is one inline test, `fortran::tests::module_and_functions` (`fortran.rs:226-236`), which only checks that `add` is extracted. The only integration mention is `detect_prefixes.rs:86`. There are no Fortran fixtures under `crates/*/tests/` and no ignored tests.
- **CHANGELOG**: `CHANGELOG.md:5469` corrected a skill claim that Fortran lacked cross-file resolution. `CHANGELOG.md:5975` notes the stdlib list is untuned.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | 7 extensions, case-folded; free-form grammar only, fixed-form parses with errors and loses sites; no .f77/.inc |
| 02 Callables extracted | partial | subroutine/function/program/module/internal procs; module prefix leaks past `end module`; no submodule/entry/stmt-fn/generic/type-bound |
| 03 Call-site forms | minimal | bare `call x()` and `f(x)` only; `obj%m()` silently dropped; arrays read as calls; no value refs |
| 04 Imports & module resolution | partial | `use m` → unqualified prefix (high when unique); no rename/only, no include, no resolution without any `use` |
| 05 Type inference & receiver typing | none | no type facts emitted |
| 06 Inheritance, dispatch & overloading | none | no type extension, generic interfaces, or dispatch membership; interface bodies compete with bodies |
| 07 Scoping & shadowing | none | no locals, no nested narrowing, case-sensitive matching |
| 08 Macros, preprocessor & codegen | none | #ifdef branches both extracted; no include/define handling |
| 09 FFI & cross-language links | none | bind(c) unread; C↔Fortran calls bind to prototypes |
| 10 Frameworks & entry points | minimal | one detect-only rule `fortran-program` (Cli) that discloses a gap, mints no nodes |
| 11 Test harness entries | none | no pFUnit/test-drive/veggies rules |
| 12 Dead-code analysis | minimal | degraded class, max medium; PROGRAM not a root (only `main`), bind(c)/public not roots |
| 13 Builtins & stdlib tables | partial | 163 lowercase intrinsics, untuned; case-sensitive so uppercase intrinsics go external; no MPI/OpenMP |
| 14 Confidence & certainty | partial | high via intra-file/unique use-prefix, medium via ≤3 fallback; several false highs |
| 15 Dynamic use & reflection | n/a | no reflection in language; procedure pointers unmodelled |
| 16 Known gaps & limitations | n/a | documented gaps: case, no-use resolution, prefix leak, `%` calls, one inline test |
