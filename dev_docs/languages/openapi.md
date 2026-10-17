# openapi

The OpenAPI / Swagger plugin (`OpenApiPlugin`, `crates/cgg-lang/src/plugins/openapi.rs`, 225 lines) is one of cgg's five interface/descriptor languages. It maps a REST API document's shape graph onto the call-graph model. Each reusable component (schema, parameter, requestBody, response, header) and each HTTP operation becomes a callable. Each `$ref` pointer becomes a call site, attributed by byte containment to the component or operation it sits inside. There is no grammar of its own: YAML and JSON documents are both parsed with the workspace `tree-sitter-yaml` crate (`Cargo.toml:129` `tree-sitter-yaml = "0.7"`, `openapi.rs:43-45`), because JSON is flow-style YAML. The tree walking lives in the shared helper module `crates/cgg-lang/src/plugins/structured.rs`, which the AsyncAPI plugin also uses. The plugin does not override `resolver_kind`, `signals`, `ts_language_for` or `max_line_bytes`, so it takes the trait defaults (`crates/cgg-lang/src/lib.rs:304-324`). Resolution has no OpenAPI-specific code except one branch in `cross_file::try_resolve_ref` that matches by global simple name (`crates/cgg-resolve/src/cross_file.rs:5222-5241`).

Fixtures used below live in `<scratch>/parity/openapi/` (directories `a/`, `b/`, `e/`, `f/`). They were run with `target/release/cgg` 0.11.0, `-t json -o X.json`, and the `X.json.audit.json` sidecar was read.

## 01. Detection & parsing

Supported:

- **Extensions `.yaml` / `.yml` / `.json`, content-gated.** The plugin declares `[".yaml", ".yml", ".json"]` (`openapi.rs:37-39`). The detector never assigns these extensions by extension alone. Rule 1b (`crates/cgg-lang/src/detect.rs:76-98`) sniffs the content, and a file with no match is `Unknown` (it does not fall through to the extension rule). `detected_via = "content:openapi"`.
  - `openapi: 3.0.0` → openapi. `swagger: "2.0"` → openapi. Both map to `openapi` (`detect.rs:166-170`).
  - Test: `detect.rs:792 openapi_and_swagger_yaml_json_detect_by_content` (YAML 3.x, Swagger 2 YAML, minified JSON).
- **Sniff algorithm** (`detect.rs:160-213`):
  - It reads the first 8 KiB (`detect.rs:163`).
  - Matcher 1 looks at the first 64 lines (`detect.rs:174`) for a line-leading `key:`, with leading quotes and indentation stripped. That covers block YAML and pretty-printed JSON.
  - Matcher 2 applies when the head starts with `{`. It looks for a quoted `"openapi"` / `"swagger"` anywhere in the first 2048 bytes, char-boundary safe (`detect.rs:193`). Regression test at `detect.rs:~870` (a non-ASCII catalogue straddling byte 2048 returns `None`, no panic).
  - Fixture `b/min.json`, a one-line minified JSON document: detected `content:openapi`, analyzed.
- **Shebangs:** none (`openapi.rs:40-42`).
- **Grammar:** `tree_sitter_yaml::LANGUAGE` for every file (`openapi.rs:44`). One grammar, so there is no per-file dialect choice.
- **Depth guard:** the generic `cgg_lang::exceeds_depth(&tree, MAX_TREE_DEPTH=4000)` → `SkipReason::TooDeep` applies to every language (`crates/cgg/src/lib.rs:155, 451-455`). It matters here because `structured::collect_refs` recurses (`structured.rs:154-181`). Not exercised with an OpenAPI fixture (unverified for this language specifically).
- **Parse errors:** there is no re-parse or fallback. `parse_status` is `"error"` when the tree has an error node, and extraction still runs on the partial tree (`lib.rs:457-461`). Fixture `f/api.yaml` with an undefined alias `<<: *x` still parsed `ok` and extracted.

Not supported or partial:

- **Root key past line 64 (YAML) is not detected.** Fixture `b/late.yaml` (70 comment lines, then `openapi: 3.0.0`) was skipped as `unknown-extension`.
- **Split-spec fragments are never analyzed.** A referenced file with no root `openapi:`/`swagger:` key (for example `paths.yaml` holding only `PetById:`) is skipped as `unknown-extension` (fixture `a/paths.yaml`). Multi-file specs therefore lose every definition kept in fragment files.
- **Multi-document YAML: only the first document is read.** `structured::document_root` takes the first `document` child (`structured.rs:94-101`). Fixture `b/multi.yaml`: `First`/`Second` from document 1 extracted, `InDoc2` from document 2 missing.
- **No `max_line_bytes` cap and no minified handling.** The minified-file heuristic (`--skip-minified`) covers only `.js/.mjs/.cjs/.css` (`crates/cgg-walk/src/lib.rs:19-24`), and the plugin keeps the trait default `None` line cap.
- **Not detected:** `.yaml` / `.json` files whose root key is something else. OpenAPI embedded in other formats (Markdown, code annotations such as swagger-php or springdoc) is out of scope.

## 02. Callables extracted

Every definition is a `DefRecord` with `variant: DefVariant::FreeFunction`, empty `visibility`, no attributes, and a span covering the value node (`structured.rs:111-134`). In the JSON output its `kind` is `"function"`. `qualified_name == simple_name == key` (no owner, no file or section prefix). `signature_hint` is `"<kind> <name>"`.

Supported:

- **Schemas, OpenAPI 3.** `components: {schemas: {Pet: {...}}}` → callable `Pet`, hint `schema Pet` (`openapi.rs:62`). Test `extracts_schema_and_operation_defs`.
- **Parameters, OpenAPI 3.** `components/parameters/Limit` → `Limit`, hint `parameter Limit` (`openapi.rs:63`). Fixture `a/` produced it.
- **Request bodies.** `components/requestBodies/NewPet` → `requestBody NewPet` (`openapi.rs:64`). Fixture `a/`.
- **Responses.** `components/responses/PetList` → `response PetList` (`openapi.rs:65`). Fixture `a/`.
- **Headers.** `components/headers/Rate` → `header Rate` (`openapi.rs:66`). Fixture `a/`.
- **Swagger 2 definitions.** `definitions: {User: {...}}` → `schema User` (`openapi.rs:67`). Fixture `b/v2.yaml`: `User`, `Error`, `Thing`.
- **Operations.** `paths: {/pets: {get: {operationId: listPets}}}` → callable `listPets`, hint `operation listPets` (`openapi.rs:73-94`).
  - Methods recognised: `get put post delete options head patch trace` (`openapi.rs:26-28`). Any other key under a path item is skipped (`x-custom: {operationId: notAnOp}` was not extracted in fixture `a/`).
  - With a missing or empty `operationId`, the name is synthesized as `"<METHOD> <path>"` (`openapi.rs:79-84`): `POST /pets` (test `extracts_schema_and_operation_defs`), and `GET /a` for `operationId: ''` (fixture `f/`).
  - A dotted operationId is kept verbatim: `operationId: users.list_users` → qn `users.list_users` (fixture `b/v2.yaml`).
- **JSON documents** extract the same way (test `parses_json_documents_too`; fixture `b/min.json` gave `A`, `Dup`, `Dup`, `getA`).

Not supported or partial:

- **Swagger 2 top-level `parameters:` and `responses:` are not extracted.** Only `definitions` is read for Swagger 2. In fixture `b/v2.yaml`, `Page` and `UserList` are absent, and refs to them go to the external bucket.
- **Not extracted:** `components/securitySchemes`, `examples`, `links`, `callbacks`, and `pathItems` (3.1), plus top-level `webhooks` (3.1). In fixture `a/`, `piOp`, `webhookOp`, `CB`, `L1`, `Ex1` and `key` are all absent.
- **Operations nested in `callbacks` are not callables.** Fixture `a/`: `cbOp` was missing, and its `$ref Event` was attributed to the enclosing `getPet`.
- **Path-item `$ref`** (`/pets/{id}: {$ref: 'paths.yaml#/PetById'}`) is recorded as a reference, not a definition.
- **Names collide across sections.** `qualified_name` carries no section, so `components/schemas/Dup` and `components/responses/Dup` are two callables both named `Dup` (fixtures `b/min.json`, `e/api.yaml`). Duplicate operationIds become two same-named callables (fixture `f/`: `getA` at lines 14 and 18).
- **YAML anchors and aliases are not followed.** `Alias: *base` becomes a callable `Alias` spanning only the alias token (fixture `a/`, `Alias` lines 57-57), with no edge to `Base`.
- **No owner or kind beyond `function`:** operations are not grouped under a tag or path, and nothing is marked `abstract` or carries an attribute.

## 03. Call-site forms

The only call site is `$ref`. `structured::collect_refs` walks the whole tree from the stream root (`openapi.rs:96`, `structured.rs:154-181`). For every mapping pair whose key is `$ref`, it pushes a `RefRecord` named after the pointer's last `/` segment (`ref_simple_name`, `structured.rs:105-107`). `receiver_hint` is empty, and every other `RefRecord` field takes its default.

Supported:

- **Local component ref.** `$ref: '#/components/schemas/Pet'` → ref `Pet`. Test `ref_edges_operation_to_schema_and_schema_to_schema`.
- **Ref in array items, allOf, oneOf, nested properties.** Recorded wherever it sits. Fixture `a/` gave `Pet → Base` (allOf), `Pet → Category` (nested properties), `Category → Pet` (oneOf) and `Pets → Pet` (items).
- **Refs from operations.** Responses, requestBody and headers inside an operation all produce edges: `listPets → PetList`, `POST /pets → NewPet`, `POST /pets → Rate`.
- **Refs between components.** `PetList → Pets` and `NewPet → Pet`.
- **External-file ref.** `common.yaml#/components/schemas/Shared` → ref `Shared`, with the file part discarded. It bound cross-file in fixture `a/`.
- **URL ref.** `https://example.com/s.json#/Remote` → ref `Remote` (fixture `f/`, external bucket).
- **Whole-file ref.** `$ref: './pet.yaml'` → ref named `pet.yaml` (fixture `f/`, external bucket).
- **JSON flow-style `{"$ref": "..."}`.** Test `parses_json_documents_too`.
- **Attribution by byte containment.** Test `refs_fall_inside_their_owning_def_byte_range`.

Not supported or partial:

- **The pointer path is ignored.** `#/components/responses/Dup` cannot pick the response over a same-named schema. Fixture `e/` bound `getX` to both `Dup` callables at medium.
- **JSON-pointer escapes are not decoded.** `#/components/schemas/a~1b` stays `a~1b`, which became an external-bucket entry in fixture `a/`.
- **`$ref` keys inside literal data are treated as references.** A `$ref` under `example:` produced the edge `GET /a → Base` (fixture `f/`), although that is example payload, not a schema pointer.
- **Discriminator `mapping` strings** (`dog: '#/components/schemas/Dog'`) are not refs. Fixture `a/`: no `Dog` reference was recorded.
- **Not read:** `operationRef`, links' `operationId`, `security` requirement names and `tags`.
- **No arity, no receivers, no value refs.** None of the `RefRecord::arity`, `typed_receiver`, `path_receiver` or `value_ref` fields are set (`structured.rs:163-169`).
- **Refs outside any extracted definition have no caller.** Path-level `parameters` refs and refs inside unextracted Swagger 2 `responses` have no enclosing callable. They become `no-enclosing-callable` unresolved sites (fixture `a/`: `Limit`, line 7; `b/v2.yaml`: `User`, line 27), and a path-item `$ref` goes to the external bucket with `src: null` (fixture `a/`: `PetById`).

## 04. Imports & module resolution

Supported:

- **No import records.** The plugin writes no `imports` and no `FileFacts::packages`. Cross-file binding is by global simple name within the language. In `cross_file::try_resolve_ref`, the branch `"smithy" | "proto" | "graphql" | "openapi" | "asyncapi"` looks up `by_simple[(lang, name)]`, binds when there are 1 to 4 candidates, and otherwise returns `None` (`crates/cgg-resolve/src/cross_file.rs:5222-5241`). The resolver is `cross-file:imports` (`cross_file.rs:958`).
  - Fixture `a/`: `getPet → Shared` (in `common.yaml`) and `Shared → Pet` (back in `api.yaml`), both at medium via `cross-file:imports`.
  - Fixture `e/`: `Far` defined in 5 files gave an unresolved site with `candidates-in-other-files {candidates: 5}`.
- **Refs with no candidate anywhere** go to the per-file external bucket as `no-candidate-in-file` with `name_screen_applied: external` (fixtures `a/`: `NotThere`, `a~1b`, `PetById`; `b/`: `Page`, `UserList`). With `--include-external` they become `<external>::NotThere` leaf nodes on `ext` edges (fixture `a/`). `PetById` was not rendered, having `src: null`.

Not supported or partial:

- **The file part of a `$ref` is not used to narrow candidates.** `common.yaml#/...` and `other.yaml#/...` both match any same-named definition in any OpenAPI file.
- **Local pointers (`#/...`) are not pinned to their own document.** In fixture `a/`, `common.yaml`'s `'#/components/schemas/Pet'`, which per JSON Reference semantics names a `Pet` in `common.yaml` itself (where none exists), was bound to `api.yaml`'s `Pet`.
- **Not applicable:** `heads.rs` external-module refusal, `modules.rs` (`not-in-imported-module`) and `WorkspaceVocab` (no grep hit for `openapi` in `heads.rs` / `modules.rs` / `libscope.rs`). No manifests are read.

## 05. Type inference & receiver typing

- Not applicable. No parameters, locals, fields, return types or receivers are recorded (`structured.rs:121-133`, `163-169`), and `type_hints.rs` has no OpenAPI case (grep: no `openapi` in `type_hints.rs`).
- `cgg_lang::locals::collect` has no spec for `openapi`: the spec covers only dart, javascript, kotlin, lua, ocaml, python, rust, scala, swift, typescript and zig (`crates/cgg-lang/src/locals.rs:398-401`). `FileFacts::locals` is always empty.
- Schema-level typing (`type: object`, `format`) is not modelled.

## 06. Inheritance, dispatch & overloading

- **`allOf` / `oneOf` / `anyOf` composition** is just a `$ref` edge (`Pet → Base`, fixture `a/`). It is not recorded as a base class: no `ClassDecl`, no `trait_impl_target`.
- Not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS` (no `openapi` match in `dispatch.rs` / `type_hints.rs`). `--dynamic-dispatch` changed nothing on fixture `a/` (16 callables, 12 edges, unchanged).
- **Overloading:** duplicate operationIds or section-colliding names are separate callables. A ref to the name binds all of them (up to 4) at medium (fixture `e/`).
- **Not applicable:** `super`, `member_of_calling_class`, `overrides_below`, extension methods and mixins.

## 07. Scoping & shadowing

- Not applicable. There is no `FileFacts::locals` (Section 05) and so no `shadowed-by-local`. No lexical narrowing of nested definitions, positional shadowing or `DefRecord::conditional`.
- **Intra-file linking** is by name with byte containment: the standard `intra_file::link_file` binds a same-file single candidate at high (fixture `a/`: all 10 same-file edges were `high`, `intra-file`).
- A same-file name with two candidates (`Dup`) was not bound intra-file. It fell through to the cross-file global branch at medium (fixture `e/`, resolver `cross-file:imports`).

## 08. Macros, preprocessor & codegen

- **YAML anchors/aliases (`&base` / `*base`) and merge keys (`<<: *x`)** are the closest analogue. They are not expanded (Section 02, fixture `a/` `Alias`; fixture `f/` `<<: *x`). `structured::unwrap` skips `anchor`/`tag` children when unwrapping a node (`structured.rs:18-34`).
- **Not applicable:** macro expansion, `macro:<name>` tagging and `name_mentions` (none written).

## 09. FFI & cross-language links

- **`ffi::link_ffi`:** no OpenAPI coverage (grep: no `openapi` in `crates/cgg-resolve/src/ffi.rs`).
- **The descriptor linker excludes OpenAPI.** `descriptor::link_descriptors` covers only `DESCRIPTOR_LANGUAGES = ["proto", "graphql", "smithy"]` (`crates/cgg-resolve/src/descriptor.rs:47`). No operation → host-language handler edge is ever produced for an operationId.
  - Fixture `b/`: `operationId: users.list_users` next to `b/users.py` `def list_users()` (Python qn `users.list_users`) gave no edge between them.
  - The README says (line ~1005) that descriptor linking covers "`$ref` and shape-member edges for … OpenAPI". That holds only for `$ref` edges, which come from the ordinary intra-file and cross-file passes, not from `descriptor.rs`.
- **Markup code-behind:** not applicable.

## 10. Frameworks & entry points

- **No `RuleSpec` names `openapi` as its language.** grep for `language: "openapi"` in `crates/cgg-core/src/frameworks/rules.rs` finds nothing. In `scripts/benchmark.sh:384`, `openapi-spec` lists no frameworks.
- Two rules in other languages mention OpenAPI only in their `gap` prose:
  - `connexion` (python, kind `Network`, detect `["connexion"]`, no attributes, registrars or methods; `rules.rs:8151-8165`). Its gap explains that Connexion handlers are named by operationId strings in the OpenAPI document (`x-openapi-router-controller`, `x-*InfoFunc`), so they read as dead code. cgg does not read the spec to root them.
  - `poem` (rust, `rules.rs:2718-2730`, detect `poem`, `poem_openapi`). Its gap notes that `#[OpenApi] impl` / `#[oai(...)]` routes are not entry rules. That concerns Rust source, not OpenAPI documents.
- **No entry nodes are minted for OpenAPI operations.** Operations are not roots (dead-code `roots: 1` on fixture `a/`, and that one was the top-level `$ref`; see Section 12).
- **Not applicable:** `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for` and registrar verbs.
- `crates/cgg/tests/detect_prefixes.rs:94, 179` explicitly exempts `openapi` from the synthesized-import framework detection test ("detected by file type or by structure, which this test cannot express").

## 11. Test harness entries

- None. There is no `kind: test` rule for OpenAPI, and no notion of a test file. `TEST_CLASS_MEMBERS_ARE_ROOTS` does not apply.

## 12. Dead-code analysis

- **Classed as a descriptor language, so every finding is withheld.** `DESCRIPTOR = ["smithy", "proto", "graphql", "openapi", "asyncapi"]` (`crates/cgg-resolve/src/deadcode/caps.rs:41`) gives `LanguageClass::Descriptor` (`caps.rs:163-164`). Every finding category is then withheld with `SuppressionReason::DescriptorLanguage` (`deadcode/mod.rs:807-810`), on the reasoning that an unreferenced schema is a wire contract consumed elsewhere.
  - Tests: `deadcode/mod.rs:2017 descriptor_language_findings_are_withheld_with_a_count` and `caps.rs:241 descriptor_languages_are_classified_apart`.
  - Fixture `a/` `--dead-code`: `0 shown`. Withheld: never-referenced 6, reachable-only-from-dead-code 7, dead-cycle 2, all `(descriptor language)`.
  - The signal row reads `openapi 2 files 16 callables, class descriptor, vis no, attrs no, refs no, disp no, reach 6%`.
- **Roots:** only `toplevel:invocation`. A `$ref` with no enclosing callable roots its target: fixture `a/`'s JSON report lists root `Limit`, kind `top-level-invocation`, rule `toplevel:invocation`, "called from module top level at a/api.yaml:7". Operations are not treated as public or exported roots.
- **Signals:** none (default `PluginSignals`, `lib.rs:304-306`). No visibility, attributes, value refs or dispatch. There is no `BLIND_SPOTS` entry for openapi (`caps.rs:44-68`).
- **Not applicable:** the `implements-external-contract`, `macro-generated` and `reflection` caps (moot, since every finding is withheld).

## 13. Builtins & stdlib tables

- **There is no `crates/cgg-core/src/stdlib/openapi.txt`.** The directory holds 30 lists, none for a descriptor language. No `openapi` appears anywhere in `crates/cgg-core/src/` outside the two `rules.rs` gap strings.
- No `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` / implicit-package entries.
- `--include-stdlib` had no effect on fixture `a/` (`0 stdlib`).
- OpenAPI's built-in types (`string`, `integer`, …) are `type:` values, never `$ref`s, so no filter is needed.
- Gap: remote "well-known" refs (for example a JSON Schema meta-schema URL) are not recognised. They go to the external bucket like any other unknown name (fixture `f/` `Remote`).

## 14. Confidence & certainty

- **High:** same-file single candidate via `intra-file` (fixture `a/`: 10 high edges; fixture `f/`: `Base → Base` self-reference high).
- **Medium:** every cross-file binding (global simple-name branch, ≤4 candidates). Same-file ambiguous names fall to that branch too (fixture `e/`: `Dup` ×2 medium).
- **Unresolved:** more than 4 candidates (`candidates-in-other-files`).
- **Never low:** the low-confidence descriptor linker excludes openapi (Section 09).
- **Mermaid:** the cross-file medium edges carry the `?` marker (fixture `a/` default mermaid: `N14 -->|?| N15` and `N15 -->|?| N0`), and same-file edges are plain `-->`.
- **High can be wrong:** a high edge is not proof the pointer is right. A literal `$ref` inside `example:` gave a high edge (fixture `f/` `GET /a → Base`), because the section path is not checked.

## 15. Dynamic use & reflection

- Not applicable. `dynuse` covers Python, JS, Java, C#, Ruby and PHP only (no `openapi` in `plugins/dynuse.rs` per grep). OpenAPI has no eval or reflection. The language's "dynamic" construct is the operationId string naming a host handler (Connexion), which is not followed (Sections 09 and 10).

## 16. Known gaps & limitations

These were found while cataloguing. Each was confirmed by a fixture or by code.

1. **No operationId → host handler link.** The descriptor linker excludes openapi (`descriptor.rs:47`), and the `connexion` rule gap (`rules.rs:8163`) states Connexion handlers read as dead code. Fixture `b/`.
2. **The `$ref` pointer path and file are discarded.** Only the last segment is kept (`structured.rs:105-107`), so these all happen:
   - schema/response/parameter names collide (fixture `e/` `Dup` bound twice);
   - local `#/...` pointers bind into other files (fixture `a/` `Shared → Pet`);
   - external-file refs bind to any same-named definition;
   - JSON-pointer escapes `~0`/`~1` are not decoded (fixture `a/` `a~1b`).
3. **Swagger 2 top-level `parameters` / `responses` are not extracted** (`openapi.rs:61-70` reads only `definitions`). Fixture `b/v2.yaml`.
4. **OpenAPI 3.1 `webhooks` and `components/pathItems`, callback operations, `securitySchemes`, `links` and `examples` are not extracted.** Fixture `a/`.
5. **Split-spec fragment files without a root `openapi:` key are skipped** (`unknown-extension`), and the root key must appear in the first 64 lines (`detect.rs:174`). Fixtures `a/paths.yaml` and `b/late.yaml`.
6. **Only the first YAML document of a multi-document stream is read** (`structured.rs:94-101`). Fixture `b/multi.yaml`.
7. **`$ref` inside literal `example` / `examples` data is recorded as a reference** (`collect_refs` walks the whole tree). Fixture `f/`.
8. **Refs outside an extracted definition** (path-level `parameters`, path-item `$ref`) have no caller: `no-enclosing-callable` or `src: null` external. A side effect is that they root their targets for dead code (fixture `a/` `Limit`).
9. **YAML anchors, aliases and merge keys are not resolved.** Fixture `a/` `Alias`.
10. **Discriminator `mapping`, `operationRef` and links' `operationId` are not edges.**
11. **The global-name fan-out cap is fixed at 4** (`cross_file.rs:5235`) and is independent of `--fanout-cap`.
12. **Dead-code is entirely withheld** for the language (by design, `caps.rs:41`).

- No `gap_for()` text, `#[ignore]` tests or TODO comments exist in `openapi.rs` / `structured.rs` (grep).
- CHANGELOG 0.3-era entries (`CHANGELOG.md:5946-5966`) describe the initial plugin. No later OpenAPI-specific follow-up was found.
- README benchmark row: `OpenAPI-Specification | openapi | 132 | 334 | 0% | 47ms` (`README.md:1357`). Cross-file 0% on the corpus.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | Content-sniffed YAML/JSON via the YAML grammar; misses root key past line 64, fragment files and later YAML documents |
| 02 Callables extracted | partial | Components (3.x) + Swagger 2 `definitions` + 8 HTTP methods; no Swagger 2 params/responses, webhooks, callbacks, pathItems; names collide across sections |
| 03 Call-site forms | partial | `$ref` only, by last pointer segment; example-data `$ref`s counted, no discriminator/operationRef, no pointer-escape decoding |
| 04 Imports & module resolution | minimal | Global simple-name match (≤4 candidates), file part and local `#` scope ignored; no imports/packages |
| 05 Type inference & receiver typing | n/a | No receivers, params or locals |
| 06 Inheritance, dispatch & overloading | n/a | allOf/oneOf are plain `$ref` edges; not in INHERITANCE/OVERLOAD lists |
| 07 Scoping & shadowing | n/a | No locals; byte-containment attribution only |
| 08 Macros, preprocessor & codegen | none | YAML anchors/aliases/merge keys not expanded |
| 09 FFI & cross-language links | none | Excluded from `descriptor::link_descriptors` (proto/graphql/smithy only); no operationId → handler edge |
| 10 Frameworks & entry points | none | No rule for openapi; Connexion gap names the missing spec→handler rooting |
| 11 Test harness entries | n/a | No tests in the language |
| 12 Dead-code analysis | minimal | Descriptor class: all findings withheld; only `toplevel:invocation` roots |
| 13 Builtins & stdlib tables | n/a | No stdlib list; built-in types are never refs |
| 14 Confidence & certainty | partial | High intra-file single candidate, medium cross-file (`?` in mermaid); high possible on example-data refs |
| 15 Dynamic use & reflection | n/a | No dynuse; operationId strings not followed |
| 16 Known gaps & limitations | n/a | 12 confirmed gaps listed above |
