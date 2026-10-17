# smithy

Smithy is an interface-definition language (IDL): it has no functions, only *shapes*. The plugin (`crates/cgg-lang/src/plugins/smithy.rs`, 372 lines, `SmithyPlugin`, id `"smithy"`) maps the Smithy shape graph onto cgg's callable/edge model. Every top-level shape becomes a callable named `namespace#Shape`, and every shape reference in a shape body becomes a call site (smithy.rs:1-22). The grammar is **vendored**: `parser.c` and `tree_sitter/parser.h` come from the `tree-sitter-smithy` 0.0.1 crate (ABI 14, MIT). The published crate pins `tree-sitter 0.20` and the deprecated `language()` API (`crates/cgg-lang/vendor/smithy/PROVENANCE.md`), so `crates/cgg-lang/build.rs:20-27` compiles the C parser and smithy.rs:30-34 binds the raw `tree_sitter_smithy()` symbol through `tree_sitter_language::LanguageFn`. `grammar.js` is kept for reference only and is not compiled. `LanguagePlugin` has no `resolver_kind` method in the current trait (`crates/cgg-lang/src/lib.rs:296-341`). Smithy's resolution is not a plugin property. It comes from three things: the `"smithy"` arm of the descriptor-language by-name block in `cgg-resolve/src/cross_file.rs:5222-5242`, `DESCRIPTOR_LANGUAGES` in `cgg-resolve/src/descriptor.rs:47`, and the dead-code `DESCRIPTOR` class in `cgg-resolve/src/deadcode/caps.rs:41`. The README language table describes it as "namespace shapes (global)" (README.md:526).

Fixtures used below live in `<scratch>/parity/smithy/` (dirs `a`–`g`). They were run with `target/release/cgg` (0.11.0 build in the tree).

## 01. Detection & parsing

- **Extension**: `.smithy` only (smithy.rs:69-71). Test `plugin_loads` (smithy.rs:313). Audit for fixture `a` shows `"detected_via": "extension:.smithy"`.
- **Shebangs**: none (`&[]`, smithy.rs:72-74).
- **Content detection**: none. `crates/cgg-lang/src/detect.rs` has no smithy rule (grep for `smithy` in detect.rs and cgg-walk returns nothing). The JSON AST form of Smithy models (`*.json` with `"smithy": "2.0"`) is **not** detected. Such files go to whatever claims `.json`, which is the OpenAPI/AsyncAPI content detector only when the root key matches.
- **Grammar**: a single grammar from vendored `parser.c` (build.rs:20-27, `cargo:rerun-if-changed` on parser.c and parser.h). There are no dialects, so the default `ts_language_for` applies (lib.rs:322).
- **Re-parse / fallback**: none. A file with parse errors is still walked: tree-sitter error recovery keeps the well-formed `shape_statement`s. Fixture `c/x.smithy` reports `parse_status: "error"` in the audit, but `Holder`, `When` and `Doc` are still extracted.
- **Grammar requires a namespace**: `shape_section` is `seq(namespace_statement, use*, shape_statements?)` (grammar.js:371-391). A file with shapes but no `namespace` line yields **zero callables**. Fixture `g/nons.smithy` (`structure NoNs { x: One }`) has `parse_status: error` and no callables.
- **Skip conditions**: the generic `cgg_lang::exceeds_depth` against `MAX_TREE_DEPTH` (4,000) applies, skipping the file as `SkipReason::TooDeep` (crates/cgg/src/lib.rs:155, 451-454). `max_line_bytes` is not overridden (default `None`, lib.rs:313), so no long-line skip. Minified-file handling (locals skipped for lines over 2,000 bytes) does not matter because the plugin records no locals.
- **`$version` control statement**: optional, ignored. `g/p.smithy` has no `$version` and parses `ok`. `metadata` statements are parsed and ignored (the walker only reacts to `shape_statement`, smithy.rs:141-155).

## 02. Callables extracted

Every `shape_statement` whose `body` field has a `name` becomes one `DefRecord` (smithy.rs:157-201):

- `variant: DefVariant::FreeFunction`. The JSON output gives `kind: "function"` and the audit gives `freefunction`, for every shape kind.
- `qualified_name = "<namespace>#<Name>"` (Smithy's absolute shape id), or the bare `Name` when the namespace is empty (smithy.rs:174-178). In practice an empty namespace never extracts anything (see 01).
- `simple_name = Name`. The `signature_hint` is `"<kind> <Name>"`, where kind is the body node kind with `_statement` stripped: `service`, `resource`, `operation`, `structure`, `union`, `list`, `map`, `enum` (also for intEnum), `simple_shape` (smithy.rs:170-173, 191). Fixture `a`: `"signature_hint": "service Weather"`.
- Span: the whole `shape_statement` including leading traits (smithy.rs:179-190).
- `visibility` empty, `attributes` empty (smithy.rs:192-193).

Supported shape kinds (fixture `a/main.smithy`, all extracted):

- `service Weather { … }` → `example.weather#Weather`
- `resource City { … }` → `example.weather#City`
- `operation GetCity { … }` → `example.weather#GetCity`
- `structure GetCityInput { … }`, including `@mixin structure PageMixin {}` and `@error structure …`
- `union U { a: CityId, b: Token }`
- `list CityList { member: CityCoordinates }`
- `map M { key: Token, value: CityCoordinates }`
- `enum Kind { A, B = "b" }` and `intEnum IK { ONE = 1 }`
- Simple shapes: `string CityId`, `timestamp When`, `document Doc` (fixtures `a`, `c`)
- Tests: `extracts_all_shapes_namespace_qualified` (smithy.rs:319) covers service, operation, structure and simple shape.

NOT extracted / partial:

- **Members are not callables**: structure/union members, enum members, and list/map `member`/`key`/`value` are only reference sources. There are no `Shape$member` nodes.
- **Inline structures** (`input := { … }`, `output := for City with [PageMixin] { … }`) do not become their own shapes (Smithy would name them `ListCitiesInput`/`ListCitiesOutput`). Their member references are attributed to the enclosing operation. Fixture `a`: `ListCities -> Token` (line 35) and `ListCities -> CityList` (line 38).
- **`bigInteger` / `bigDecimal` simple shapes are dropped**: the vendored grammar spells the keywords `big_integer`/`big_decimal` (grammar.js `simple_type_name`). Fixture `c/x.smithy` has `bigInteger Big` and `bigDecimal Dec`, neither is extracted, and the file reports `parse_status: error`.
- **IDL 1.0 `set` shape is not supported**: the grammar has no `set_statement`. Fixture `e/v1.smithy` (`set MySet { member: Item }`) extracts only `Item`. The smithy.rs:13 doc comment mentions `set`, but no grammar node exists for it.
- **`apply` statements** (`apply GetCity @documentation("x")`) produce no definition and no reference. The walker only matches `shape_statement` (smithy.rs:142), and `apply_statement` is a sibling choice (grammar.js:422-424).
- **Shapes in a namespace-less file**: not extracted (01).
- Overloads/clauses, constructors, closures, nested defs, accessors, operators: not applicable (Smithy has none).

## 03. Call-site forms

References are `RefRecord { name, site_line, site_byte }` with an empty `receiver_hint`, collected only inside the shape **body** (smithy.rs:197-200, 203-244):

- **Any `shape_id` node in the body** → one reference named by `simple_shape_name` (smithy.rs:122-128). This covers:
  - service `operations: [GetCity]`, `resources: [City]`, `errors: [ServiceErr]` (shape ids inside `node_value`/`string_value`, grammar.js:255-258). Fixture `a`: `Weather -> GetCity/City/ServiceErr`.
  - resource `identifiers: { cityId: CityId }`, `read: GetCity`, `list: ListCities`, `resources: [Forecast]`. Fixture `a`: `City -> CityId/GetCity/ListCities/Forecast`.
  - operation `input: X`, `output: Y` (fixture `GetCity -> GetCityInput/GetCityOutput`).
  - structure/union member targets (`coordinates: CityCoordinates`).
  - list `member:`, map `key:`/`value:` targets (fixture `CityList -> CityCoordinates`, `M -> Token/CityCoordinates`).
  - **Mixins** `with [PageMixin]`: `mixins` is a field of the body node, so the mixin ids are references. Fixture: `GetCityInput -> PageMixin`, `ListCities -> PageMixin`.
  - **Target-resource binding** `structure X for City` and `output := for City`: the `structure_resource` shape_id is a reference. Fixture: `ListCities -> City` (line 37).
- **Operation `errors: [Id, …]`**: these are bare `identifier`s, not shape_ids. A dedicated arm handles them when the parent is `operation_errors` (smithy.rs:211-217). Test `service_references_operations_and_resources` (smithy.rs:339) asserts `NoSuchResource`.
- **Absolute ids** `example.common#Other` → `Other`. **Member ids** `Shared$field` → `Shared` (smithy.rs:122-128). Fixture `a` lines 53-54: `GetCityInput -> example.common#Other` and `-> example.common#Shared`.
- **Prelude primitives skipped**: `String, Blob, Boolean, Byte, Short, Integer, Long, Float, Double, BigInteger, BigDecimal, Timestamp, Document, Unit, Primitive*` (smithy.rs:38-60, 234), including the absolute `smithy.api#String` (fixture `a` line 57: no edge). Test `prelude_primitives_are_not_references` (smithy.rs:356). The match is on the simple name, so a user's own shape called `Unit`/`Document` in their own namespace is also never referenced (by code reading, smithy.rs:233-235; unverified at runtime).
- **Traits skipped**: subtrees of kind `trait`/`applied_traits` are never descended (smithy.rs:205). This includes shape ids *inside* trait bodies: `@references([{resource: City}])` gives no `GetCityInput -> City` edge in fixture `a`. Test `trait_shape_refs_excluded` (smithy.rs:364).
- **Elided members** `$cityId` (structure, fixture `a` line 40) produce nothing: the identifier's parent is not `operation_errors`.
- **Absolute ids in operation `errors`** (`errors: [a.b#Err]`) are a parse error in this grammar (`comma_repeat($.identifier)`, grammar.js:800-806). Fixture `g/e.smithy`: `parse_status: error`. `Op -> Err` still binds through error recovery, and 2 extra external references appear. Their names were not inspected, so they are unverified, probably recovery fragments.
- Not applicable: method/qualified/chained calls, constructors, explicit generics, call operators, paren-less calls, value refs (`RefRecord::value_ref` is never set and `_ctx` is ignored, smithy.rs:81), registrar arguments, arity (`RefRecord::arity` is never set).

## 04. Imports & module resolution

- **`use` statements are ignored**: `_use_statement` is a hidden grammar rule (grammar.js:403-407) and the plugin records no `imports`. Fixture `f`: `namespace acme.x; use acme.y#Thing; structure Widget { t: Thing }`, with `Thing` defined in both `acme.y` and `other.z`. **Both** bind (`Widget -->|?| acme.y#Thing` and `Widget -->|?| other.z#Thing`).
- **Namespace is not used for resolution.** Same-file shapes bind first through the intra-file linker. Fixture `a`: `GetCityOutput -> example.weather#CityCoordinates` (high) and *not* `example.common#CityCoordinates`. Any other reference goes to the descriptor-language arm in `cross_file.rs:5222-5242`: a global `(language, simple_name)` lookup that binds **every** candidate when there are 1–4, and binds nothing at 5 or more. Fixture `d` (five `Dup`s in `n1`…`n5`) records unresolved `candidates-in-other-files {candidates: 5}`.
- Cross-file edges carry resolver `cross-file:imports` (cross_file.rs:958) even though no import was involved (fixture `a`: `GetCityInput -> example.common#Shared medium cross-file:imports`).
- **No `FileFacts::packages`**: the namespace is not recorded as a package. No manifest reading (`smithy-build.json` is not read).
- **heads.rs external-module refusal / `WorkspaceVocab`**: no smithy handling (grep for `smithy` in `heads.rs` and `modules.rs` returns nothing). **`not-in-imported-module`**: never produced.
- **Unresolved references**: a name with no candidate goes to the per-file **external** bucket (fixture `c`: `Missing` → `external_calls: 2` together with `Dup`). It appears as `<external>::Missing` under `--include-external`.
- `crates/cgg/tests/detect_prefixes.rs:94,179` explicitly exempts smithy from import-prefix framework detection ("Descriptor languages: an 'import' is a schema reference").

## 05. Type inference & receiver typing

- Not applicable: there are no variables, receivers, parameters or return types. The plugin never sets `receiver_hint`, `typed_receiver`, `arg_type`, `local_types` or `scoped_types` (smithy.rs:237-243). `type_hints` has no smithy code (grep returns nothing).
- The closest analogue: operation `input`/`output` and member targets are recorded as plain references (03), not as types.

## 06. Inheritance, dispatch & overloading

- **Mixins** (`with [M]`) are recorded only as plain references (03). No base-class record (`ClassDecl`) and no inherited members.
- **Resource hierarchy** (`resources: [Sub]`) and **service → operation** links are plain edges, not dispatch.
- Not in `INHERITANCE_LANGS`, `OVERLOAD_ARITY_LANGS` or `dispatch::fanout` (grep for `smithy` in dispatch.rs returns nothing). `--dynamic-dispatch` does not change a smithy graph (fixture `f` shows the same 3 edges with `--dynamic-dispatch --reference-edges`).
- super/base, `member_of_calling_class`, `overrides_below`, extension methods, arity narrowing: not applicable.

## 07. Scoping & shadowing

- No `FileFacts::locals`. `crates/cgg-lang/src/locals.rs` has no smithy arm (grep returns nothing), so `shadowed-by-local` is never produced.
- Scoping is "same file first, then global by name" (04). There is no lexical nesting because Smithy shapes are all top-level.
- No conditional definitions (`DefRecord::conditional` is never set).

## 08. Macros, preprocessor & codegen

- Not applicable: Smithy has no macros or preprocessor. `apply` (trait application to an existing shape) is ignored (02). No `macro:<name>` tagging and no `name_mentions`.

## 09. FFI & cross-language links

- `ffi::link_ffi`: no smithy coverage (grep returns nothing in ffi.rs).
- **Descriptor link** (`descriptor::link_descriptors`, descriptor.rs:59-157): `smithy` is in `DESCRIPTOR_LANGUAGES` (line 47) and its family is `"smithy"` (line 54). For every smithy callable with an owner, the pass links to non-descriptor callables whose lowercased simple name equals the shape name and whose owner contains the "service" name. Edges are `confidence: Low`, `Via::Descriptor("smithy")`, resolver `descriptor-linker`, and render as `?desc` in mermaid.
  - **Defect: the "service" for smithy is the namespace's first segment, not the service.** `owner_from_qn("example.weather#GetCity")` splits on the last `.`, giving owner `example` (names.rs:81-91). Fixture `a`: `example.weather#GetCity -> impl.ExampleHandler.GetCity low descriptor` links only because `examplehandler` contains `example`. A handler named `WeatherService.GetCity` would not link.
  - **Defect: not limited to operations.** The `CallableKind::Method` filter applies only to proto (descriptor.rs:105-107). Fixture `f`: `structure Widget` in `acme.x` links to `s.AcmeStore.widget` (`?desc`).
  - Name matching is a case-insensitive exact match. Snake case does not link: fixture `a` `get_city` is not linked.
  - A single-segment namespace (`namespace weather`) has no `.`, so the owner is None and nothing links (by code reading, descriptor.rs:111-113; unverified at runtime).
  - Unit tests in descriptor.rs cover only proto (`rpc_links_to_a_conventionally_named_implementation` etc.). No smithy test exists.
- Markup code-behind: not applicable.

## 10. Frameworks & entry points

- **No framework rule applies to smithy.** No `RuleSpec` in `crates/cgg-core/src/frameworks/rules.rs` has `language: "smithy"` (rule languages counted: java 60 … bash 1; smithy absent). No multi-language or wildcard rule exists. `frameworks/mod.rs` has no smithy code.
- No member_sources, signature_methods, container_attributes, markup_kinds or registrar verbs.
- `@http`, `@readonly`, `@paginated` traits are skipped (03), so operations are never entry points even though they are the API surface.
- Benchmark: `scripts/benchmark.sh:102` (`smithy-protocol-tests`, `smithy-aws-protocol-tests/model`). The APPS row at line 398 names only Java/Python rules for the same repo (its non-Smithy code).

## 11. Test harness entries

- None. No `kind: test` rule for smithy, and it is not in `TEST_CLASS_MEMBERS_ARE_ROOTS`. Smithy's own protocol-test traits (`@httpRequestTests`, `@httpResponseTests`) are traits and are skipped (smithy.rs:205), so they are not recognised as test entries.

## 12. Dead-code analysis

- Smithy is classified `LanguageClass::Descriptor` (caps.rs:41, 163-164). Test `descriptor_languages_are_classified_apart` (caps.rs, openapi case) covers the same path.
- **All smithy findings are withheld** with `SuppressionReason::DescriptorLanguage` (deadcode/mod.rs:807-810, report label "descriptor language", crates/cgg/src/deadcode/report.rs:61). Their evidence lists `LanguageIsDescriptor` (mod.rs:926). Fixture `a --dead-code`: `never-referenced 5 withheld`, `reachable-only-from-dead-code 12 withheld`, `dead-cycle 2 withheld`, `roots 0 discovered`, signal coverage `vis no / attrs no / refs no / disp no / reach 0%`.
- `signals()` is not overridden (default `PluginSignals::default()`, lib.rs:304-306), so no visibility, attribute, value-reference, export or test-tag signals are recorded.
- No roots: services are not treated as roots, and there are no public/exported roots and no caps (implements-external-contract, macro-generated, reflection do not apply).

## 13. Builtins & stdlib tables

- No `crates/cgg-core/src/stdlib/smithy.txt`, and no smithy entries in `stdlib.rs` (CAST_INTRINSICS, CONSTRUCTING_METHODS, implicit packages: none).
- The prelude list is **local to the plugin** (`PRELUDE`, smithy.rs:38-60, 21 names). These references are dropped at extraction, so they never reach the stdlib bucket and `--include-stdlib` adds nothing for smithy (fixture `a` run: `0 stdlib`).
- The prelude list is missing parts of the Smithy prelude: prelude *traits* (irrelevant, since traits are skipped), and `PrimitiveBoolean`… are listed. `smithy.api#Unit` is listed. Shapes added in later Smithy versions are not checked (unverified).

## 14. Confidence & certainty

- **high**: same-file references bound by the intra-file linker (fixture `a`: every same-file edge is `high`, resolver `intra-file`).
- **medium**: every cross-file reference, from the descriptor-language arm of `cross_file.rs:5222-5242`, even when only one candidate exists. Fixture `a`: `GetCityInput -> example.common#Shared medium`. Mermaid marks these with `?` (fixture `f`: `N0 -->|?| N1`).
- **low**: descriptor → host-language links (`?desc`), always (descriptor.rs:141).
- Same-name candidates in other namespaces all bind at medium (04). That is a wrong edge, and the medium label is accurate.

## 15. Dynamic use & reflection

- Not applicable. `dynuse.rs` has no smithy coverage, and Smithy has no runtime.

## 16. Known gaps & limitations

- `use` statements and namespaces are ignored for resolution. References bind globally by simple name to up to 4 candidates across all namespaces (fixture `f`), and 5 or more is unresolved (fixture `d`).
- The descriptor link uses the namespace's first segment as the "service" and links any shape kind, not just operations (fixtures `a`, `f`; descriptor.rs:105-123). Expect both false edges and misses.
- `bigInteger`/`bigDecimal` shapes are dropped by a grammar keyword bug (`big_integer`/`big_decimal`). Fixture `c`.
- IDL 1.0 `set` is unsupported (fixture `e`). The smithy.rs:13 comment claims `set`.
- Absolute shape ids in operation `errors: [...]` cause a parse error (grammar.js:800-806, fixture `g/e.smithy`).
- Files without a `namespace` line yield nothing (fixture `g/nons.smithy`).
- `apply` statements, trait bodies (`@references`, `@http(uri…)`, protocol-test traits) and elided `$member`s produce no edges or nodes.
- Inline `input :=`/`output :=` structures are folded into the operation and get no `XInput`/`XOutput` node.
- Members are not nodes. A `Shape$member` reference collapses to `Shape`.
- The JSON AST model format is not detected.
- The prelude skip is by simple name, so user shapes named like prelude types are never referenced.
- Unit test coverage is limited to the four tests in smithy.rs (extraction only). There is no resolve-level or descriptor-link smithy test. `crates/cgg-py/tests/test_analyze.py:552` only asserts the language is listed. No ignored tests.
- CHANGELOG: introduced with the descriptor plugins (CHANGELOG.md:5947-5952). The benchmark row was added later (CHANGELOG.md:5481-5484). README row 1354: `smithy/protocol-tests | 827 | 1,374 | 49%`.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.smithy` only; no JSON AST detection; namespace-less files and some keywords fail to parse |
| 02 Callables extracted | partial | all IDL 2.0 top-level shapes as FreeFunction `ns#Name`; no members, inline structs, `set`, bigInteger/bigDecimal |
| 03 Call-site forms | partial | every body shape_id + operation errors + mixins + `for`; traits, apply, elided members skipped |
| 04 Imports & module resolution | minimal | `use`/namespace ignored; global by-name ≤4 candidates |
| 05 Type inference | n/a | no variables or receivers |
| 06 Inheritance & dispatch | minimal | mixins only as plain edges; no dispatch membership |
| 07 Scoping & shadowing | minimal | same-file first, then global; no locals |
| 08 Macros & codegen | n/a | no macros; `apply` ignored |
| 09 FFI & cross-language | partial | descriptor link exists but keys on namespace head and is not operation-only (low conf) |
| 10 Frameworks & entry points | none | no rules; protocol traits skipped |
| 11 Test harness entries | none | no test rules; protocol-test traits skipped |
| 12 Dead-code analysis | minimal | Descriptor class: every finding withheld; no roots or signals |
| 13 Builtins & stdlib | partial | 21-name in-plugin prelude skip list; no stdlib file |
| 14 Confidence | partial | same-file high, cross-file medium, descriptor low; namespace-blind |
| 15 Dynamic use | n/a | no runtime |
| 16 Known gaps | n/a | 13 gaps listed above |
