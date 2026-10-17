# graphql

The GraphQL plugin (`GraphqlPlugin`, id `"graphql"`, `crates/cgg-lang/src/plugins/graphql.rs`, 235 lines including tests) reads GraphQL **SDL** (schema definition language) only. It uses the crates.io grammar `tree-sitter-graphql` 0.1.0 (`Cargo.toml:126` declares `"0.1"`; `Cargo.lock:1538-1541` resolves 0.1.0; `crates/cgg-lang/Cargo.toml:63`; upstream is `joowani/tree-sitter-graphql`). Nothing is vendored. The plugin is registered at `crates/cgg-lang/src/plugins.rs:166` (module declared at `plugins.rs:19`).

The walker is a single pass. Six type-definition node kinds become `DefVariant::FreeFunction` definitions whose qualified name is the bare type name (`graphql.rs:25-33`, `96-126`). Every `named_type` inside such a definition becomes an owner-less bare reference, except the five built-in scalars (`graphql.rs:23`, `128-154`).

- **Resolver kind**: the `LanguagePlugin` trait has no `resolver_kind` method in this tree (`crates/cgg-lang/src/lib.rs:296-326`), so there is none to report. Resolution goes through the generic intra-file linker and then the descriptor-language arm of `cross_file::resolve`: a global by-simple-name match bounded to ≤4 candidates (`crates/cgg-resolve/src/cross_file.rs:5223-5243`).
- **Descriptor membership**: GraphQL is listed as a descriptor language in `crates/cgg-resolve/src/descriptor.rs:47` and in the dead-code `DESCRIPTOR` class (`crates/cgg-resolve/src/deadcode/caps.rs:41`).
- **Trait defaults**: the plugin declares no `signals()`, `max_line_bytes()` or `ts_language_for()` override, so all three take the defaults (`lib.rs:304-324`).

Fixtures used below live under `<scratch>/parity/graphql/`:

- `t1`: full SDL + operations + extension files;
- `t2`: Go/TS resolvers next to the SDL;
- `t3`: parse error;
- `t4`: legacy comma `implements`;
- `t5`: 5-way and 4-way duplicate types.

## 01. Detection & parsing

- **Extensions `.graphql`, `.gql`, `.graphqls`** (`graphql.rs:42-44`).
  - Fixture `t1`: `schema.graphql` gives `detected_via: extension:.graphql`, `other.gql` gives `extension:.gql`, `ext.graphqls` gives `extension:.graphqls`.
  - Uppercase `Up.GRAPHQL` was also detected as `extension:.graphql`, because the extension match falls back to case-insensitive (`crates/cgg-lang/src/detect.rs:101-127`).
- **Shebangs**: none (`graphql.rs:45-47`).
- **Content detection**: none. Content sniffing in `detect.rs` covers only `.yaml`/`.yml`/`.json` (OpenAPI/AsyncAPI, `detect.rs:77-98`, `160`) and `.h`.
  - A `.json` introspection result (`__schema`) is not detected as GraphQL. This follows from the code and was not run.
  - SDL embedded in host-language strings (`gql\`type X {...}\`` in JS/TS, `typeDefs` strings) is not parsed. No code path extracts it (grep for `gql` in `crates/cgg-lang/src` matches only the `.gql` extension at `graphql.rs:43`).
- **Grammar**: a single grammar, `tree_sitter_graphql::LANGUAGE` (`graphql.rs:48-50`). It parses both type-system documents and executable documents (`operation_definition`, `fragment_definition` and the `*_extension` kinds all exist in the grammar's `node-types.json`).
- **Re-parse / fallback**: none. A file with a syntax error is extracted from the error-recovered tree.
  - Fixture `t3`: in `type A { b: B` (missing `}`) followed by `type B { c: C }`, the definition `B` **is lost entirely**. `A` spans lines 1-2, and `A -> C` is emitted, because B's field was attributed to A. `parse_status: error`.
  - Fixture `t4`: the legacy comma form `type D implements X, Y` is a parse error in this grammar. Its `implements_interfaces` rule accepts only `&` (grammar.js:133-137), and comma is an `extra`. Only `D -> X` is emitted; **`Y` is dropped**.
- **Skip conditions**:
  - The generic depth gate applies: `cgg_lang::exceeds_depth` against `MAX_TREE_DEPTH = 4000` produces `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155`, `451-454`).
  - No `LongLine` cap: `max_line_bytes` is the default `None`.
  - Minified: the 2,000-byte line rule gates only the locals walk, and GraphQL records no locals. Not applicable.
- **Tests**:
  - `graphql.rs:187` `plugin_loads`.
  - `crates/cgg/tests/detect_prefixes.rs:94` and `:179` deliberately skip graphql. A descriptor language has "no import statement to synthesize".

## 02. Callables extracted

- **`type Name { ... }`** (`object_type_definition`) becomes a node with `signature_hint` `type Name` (`graphql.rs:27`, `119`). Test `extracts_type_defs` (`graphql.rs:193`).
- **`interface Name { ... }`** becomes a node with hint `interface Name` (`graphql.rs:28`). Fixture `t1`: `Node`, `Named`.
- **`input Name { ... }`** becomes a node with hint `input Name` (`graphql.rs:29`). Fixture `t1`: `CityInput`, `NestedInput`.
- **`union Name = A | B`** becomes a node with hint `union Name` (`graphql.rs:30`). Fixture `t1`: `SearchResult`.
- **`enum Name { A B }`** becomes a node with hint `enum Name` (`graphql.rs:31`). Fixture `t1`: `Role`.
- **`scalar Name`** becomes a node with hint `scalar Name` (`graphql.rs:32`). Fixture `t1`: `DateTime`.
- **Shape of every node**:
  - variant `DefVariant::FreeFunction` (`graphql.rs:114`); JSON kind `function`, audit kind `freefunction` (fixture `t1`);
  - `qualified_name == simple_name ==` the type name. There is no owner or namespace, because "GraphQL has no namespaces" (`graphql.rs:6-7`, `111-113`);
  - `visibility` and `attributes` are always empty (`graphql.rs:120-121`).
- **Duplicates**: the same type name in two files produces two nodes with the same qualified name. Fixture `t1`: `Ext` appears in both `ext.graphqls` and `Up.GRAPHQL`.
- **Not extracted (no node)**:
  - **Fields** (`field_definition`): `Query.city` is not a callable. Fields only contribute references from their type. Fixture `t1`: no `Query.city` node exists.
  - **Field arguments, input fields and enum values** (`enum_value_definition`).
  - **`directive @auth(...) on FIELD_DEFINITION`** (`directive_definition` is not in `DEF_KINDS`, `graphql.rs:26-33`). Fixture `t1`: no `auth` node.
  - **`schema { query: Query mutation: Mutation }`** (`schema_definition`).
  - **Type-system extensions**: `extend type`, `extend interface`, `extend union`, `extend input`, `extend enum`, `extend scalar` and `extend schema` (the `*_extension` kinds are absent from `DEF_KINDS`). The walker never enters them either, because `walk` only records `DEF_KINDS` and otherwise only recurses (`graphql.rs:80-94`). Fixture `t1`: `extend type City { mayor: Person }` produced no `City -> Person` edge.
  - **Executable documents**: `query`, `mutation`, `subscription` and `fragment` definitions. Fixture `t1`: `ops.graphql` analyzed with `parse_status: ok` but contributed 0 callables and 0 references.
  - **Descriptions** (`"""..."""`) are ignored.
- **Not applicable**: methods, constructors, closures, nested defs, accessors, operators, overloads and generated code have no GraphQL counterpart.

## 03. Call-site forms

The only "call" is a type reference: a `named_type` node inside a recorded definition (`graphql.rs:128-154`). Each produces a `RefRecord` carrying only the name, an empty `receiver_hint`, the site line and the site byte (`graphql.rs:134-140`).

- **Field result type**: `city(...): City`. Test `field_implements_and_union_references` (`graphql.rs:214`); fixture `t1`: `Query -> City`.
- **Wrapped types**: `[Coord!]!` and `[[D!]]!` reach the inner `named_type` through recursion. Fixture `t4`: `Q -> D`; fixture `t1`: `City -> Coord`.
- **Argument / input types**: `city(filter: CityInput, at: DateTime)` (`graphql.rs:220`). Fixture `t1`: `Query -> CityInput` and `Query -> DateTime` (custom scalar).
- **Input field types**: `input CityInput { nested: NestedInput }`. Fixture `t1`: `CityInput -> NestedInput`.
- **`implements A & B`**: fixture `t1`: `City -> Node` and `City -> Named`.
- **Interface implementing interface**: `interface Named implements Node`. Fixture `t1`: `Named -> Node`.
- **Union members**: `union SearchResult = City | Coord`. Fixture `t1`: `SearchResult -> City` and `SearchResult -> Coord`.
- **Enum-typed field**: `role: Role`. Fixture `t1`: `City -> Role`.
- **Self reference**: `type Person { friend: Person }` produces a self-loop `Person -> Person` at high (fixture `t1`).
- **Built-in scalars are not references**: `String`, `Int`, `Float`, `Boolean` and `ID` are filtered before the record is made (`graphql.rs:23`, `133`). Test `builtin_scalars_excluded` (`graphql.rs:225`).
- **Not recorded**:
  - **Directive applications**: `@auth(role: USER)` is a `directive`, not a `named_type`. Fixture `t1`: there is no reference to `auth` and none to the enum value `USER`.
  - **Default values**: `= ADMIN`, `= {x: 1}`.
  - **Directive-definition argument types**: `directive @auth(role: Role)` is not inside a recorded definition.
  - **Root operation types** in `schema { query: Query }`. Fixture `t1`: `Query` and `Mutation` have no incoming edge and are listed by `--report-unreferenced`.
  - **References inside `extend ...` blocks** (see 02).
  - **Executable documents**: variable types (`$f: CityInput`), fragment type conditions (`on City`), inline fragments, and field selections against schema fields.
- **Not applicable**: method, qualified, chained, constructor, explicit-generic, operator, paren-less, value-ref and registrar shapes. There is also no arity recording (`RefRecord::arity` is left at default).

## 04. Imports & module resolution

- **No import forms**: GraphQL SDL has no import statement, so the plugin emits no import records. Grep of `graphql.rs` finds no `imports` push.
  - Tool-specific `# import "x.graphql"` comments (graphql-import) are comments in the grammar and are not read. This follows from the code: comments are extras (grammar.js:4).
- **Cross-file linking**: global by-simple-name match within the language (`cross_file.rs:5223-5243`, the `"smithy" | "proto" | "graphql" | "openapi" | "asyncapi"` arm).
  - At 1–4 same-named candidates, binds every one of them, all at **medium**, with resolver `cross-file:imports`. Fixture `t1`: `Ext -> Person` and `Person -> City` are medium. Fixture `t5`: 4 copies of `P` give 4 medium edges; mermaid draws each as `-->|?|`.
  - At more than 4 candidates, binds nothing. The audit records `candidates-in-other-files {candidates: 5}` in `unresolved_calls`, not `fanout-cap-exceeded` (fixture `t5`).
  - The cap of 4 is hard-coded. `--fanout-cap 0` does not disable this binding: fixture `t5` still emitted 4 edges. This differs from the documented meaning of `--fanout-cap 0` for other languages ("never guess").
  - **No candidate anywhere**: `ghost: Missing` is recorded in the per-file `external_calls` bucket with stage `no-candidate-in-file` and `name_screen_applied: external` (fixture `t1`, audit for `other.gql`). With `--include-external` this renders as `<external>::Missing`.
- **Not applicable**: `FileFacts::packages`, manifest reading, `heads.rs` external-module refusal, `modules.rs` `not-in-imported-module` and workspace vocab. GraphQL appears in none of them (grep of `crates/cgg-resolve/src/{heads,modules,libscope}.rs` for `graphql` finds nothing).
- **No scoping by schema project**: the match is global per language, so two unrelated schemas in one tree that share a type name cross-link (fixture `t5`).

## 05. Type inference & receiver typing

- Not applicable. References are type names themselves and carry no receiver. `type_hints`, `declared_type`, closure types, implicit receivers and weak receiver types are never involved: grep of `crates/cgg-resolve/src/type_hints.rs` for `graphql` finds nothing.
- Nullability and list wrappers (`!`, `[...]`) are handled structurally, by recursing to the inner `named_type` (see 03), not through `declared_type`.

## 06. Inheritance, dispatch & overloading

- **`implements` is a plain reference**: `City -> Node`, from implementer to interface (fixture `t1`). No base-class record (`ClassDecl`) is produced, so `resolve_via_bases`, `member_of_calling_class` and `overrides_below` never see GraphQL.
- **`--dynamic-dispatch` / `inheritance_fanout`**: graphql is not in `INHERITANCE_LANGS` (grep finds no `graphql` in `crates/cgg-resolve/src/dispatch.rs`). Fixture `t1` with `--dynamic-dispatch --reference-edges` produced the same 18 edges, all `via: direct`, and no `Node -> City` interface-to-implementer edges.
- **Union membership** is likewise a forward reference only (`SearchResult -> City`).
- **Not applicable**: overloading and arity narrowing (not in `OVERLOAD_ARITY_LANGS`), super calls, extension methods and mixins.

## 07. Scoping & shadowing

- Not applicable. The plugin records no `FileFacts::locals` (no `locals` push in `graphql.rs`), and GraphQL has no lexical scopes.
- Same-file references bind through the intra-file linker at **high**, resolver `intra-file` (fixture `t1`: 15 high intra-file edges). Cross-file binding happens only when no same-file definition exists.
- No conditional definitions (`DefRecord::conditional` is never set).

## 08. Macros, preprocessor & codegen

- Not applicable. There are no macros or preprocessor directives.
- Directives (`@deprecated`, custom directives) are neither expanded nor recorded (see 02/03).
- No `name_mentions`.
- Code generated from SDL (graphql-codegen, gqlgen, Apollo codegen) is not linked back to the SDL.

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: not applicable to GraphQL (grep of `crates/cgg-resolve/src/ffi.rs` for `graphql` finds nothing).
- **`descriptor::link_descriptors`**: GraphQL is in `DESCRIPTOR_LANGUAGES` (`descriptor.rs:47`) with family `"graphql"` (`descriptor.rs:53`). **In practice it can never fire for GraphQL.**
  - An operation must have an owner: `owner_from_qn(&op.qualified_name)` must return `Some` (`descriptor.rs:105-107`).
  - Every GraphQL qualified name is a bare type name with no owner (`graphql.rs:111-113`), and fields are not extracted at all.
  - Fixture `t2`: an SDL `type Query { city: City }` next to Go `func (r *queryResolver) City(...)`, Go `QueryResolver.Query` and TS `class CityResolver { city() }` produced only `Query -> City`. There is no `Via::Descriptor` edge.
  - `crates/cgg-core/src/graph.rs:86-88` documents `Via::Descriptor` as linking "a GraphQL field to its resolver", and README:1005-1006 lists GraphQL under descriptor linking. Both describe behaviour the GraphQL plugin cannot produce today.
- **Markup code-behind**: not applicable.
- **Host-language GraphQL frameworks** are covered on the host side only (see 10). Nothing links a Rust `#[Object] impl Query { fn city }` or an Apollo resolver map to the SDL `Query` type.

## 10. Frameworks & entry points

No `RuleSpec` in `crates/cgg-core/src/frameworks/rules.rs` has `language: "graphql"`: grep for `"graphql"` in that file finds no language field. No framework entry node is ever minted from a `.graphql` file. Fixture `t1` with default entry nodes shows no `<framework-entry>` node.

GraphQL-server rules that exist for **host** languages, for the comparison pass:

- **`async-graphql`** (rust, kind `Network`):
  - detect: `async_graphql` (`rules.rs:9251-9266`);
  - container attributes `Object`, `ComplexObject`, `Subscription`, `MergedObject` and `Interface` root every member of the impl (`rules.rs:11777-11783`, `container_attributes_for`).
- **`juniper`** (rust, `Network`):
  - detect: `juniper` (`rules.rs:9268-9284`);
  - container attributes `graphql_object`, `graphql_subscription` and `graphql_interface` (`rules.rs:11784-11788`).
- **`graphql-yoga`** (typescript and javascript, `Network`):
  - detect: `graphql-yoga`, `@graphql-yoga/node`, `@graphql-tools/schema` (`rules.rs:8476-8513`);
  - a gap-only rule: "cgg has no entry rules for GraphQL Yoga resolvers ... every resolver reads as dead code" (`rules.rs:8492`, `8513`).
- **`apollo-server`** (typescript and javascript, `Network`): a gap-only rule with the same resolver-map limitation (`rules.rs:8538`, `8563`).

None of the following applies to graphql: `member_sources_for`, `signature_methods_for`, `markup_kinds_for` or registrar verbs.

## 11. Test harness entries

- None. No `test`-kind rule applies to graphql, and `TEST_CLASS_MEMBERS_ARE_ROOTS` does not apply.
- GraphQL has no test construct. Operation documents used as test fixtures are not extracted (see 02).

## 12. Dead-code analysis

- **Language class `Descriptor`** (`caps.rs:38-41`, `163-164`). Every dead-code finding for a descriptor language is withheld with `SuppressionReason::DescriptorLanguage` (`crates/cgg-resolve/src/deadcode/mod.rs:806-809`), and the finding carries `Evidence::LanguageIsDescriptor` (`mod.rs:926`).
  - Test: `descriptor_language_findings_are_withheld_with_a_count` (`mod.rs:2017`, with an openapi node).
  - Fixture `t1` `--dead-code`: `graphql never-referenced 5 withheld (descriptor language)` and `reachable-only-from-dead-code 10 withheld`, with 0 findings shown.
  - The capability row is `class: descriptor`, with visibility/attributes/value_references/dispatch/exports/test_tagging all `none`, `max_confidence: medium`, 0 roots and `root_rules_active: []`.
- **Roots**: none. No public/exported concept and no framework or test roots. `schema { query: Query }` is not treated as a root (see 03).
- **`--report-unreferenced`** does **not** withhold descriptor languages. Fixture `t1` listed `Ext` ×2, `Mutation`, `Query` and `Unused` under "unreferenced, and no root rule explains it". The root operation types and every type used only through `extend` blocks therefore appear as unreferenced.
- **Not applicable**: dead-code signals (`signals()` is default), caps (`implements-external-contract`, `macro-generated`, `reflection`) and unresolved-doubt handling.

## 13. Builtins & stdlib tables

- **No `crates/cgg-core/src/stdlib/graphql.txt`**: the directory listing has none, and `stdlib_names` has no `graphql` entry (`crates/cgg-core/src/stdlib.rs:221-257`). `--include-stdlib` therefore never yields a graphql stdlib node.
- **The built-in scalar list lives in the plugin instead**: `BUILTIN_SCALARS = ["String", "Int", "Float", "Boolean", "ID"]` (`graphql.rs:23`). These are dropped at extraction, so they appear in no bucket (not stdlib, not external, not unresolved). Fixture `t1`: `stdlib_calls: 0`.
- **Built-in directives are not modelled**: `@deprecated`, `@skip`, `@include`, `@specifiedBy`.
- **Introspection types are not filtered**: `__Schema`, `__Type`. A reference to one would go to the external bucket. This follows from the code and was not run.
- **Not applicable**: `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, implicit packages and `rust_types`.

## 14. Confidence & certainty

- **high**: a same-file type reference bound by the intra-file linker (fixture `t1`: 15 high edges, resolver `intra-file`). Self-references are high too.
- **medium**: every cross-file reference, even with exactly one candidate. Fixture `t1`: `Ext -> Person` (one `Person` in the tree) is medium with resolver `cross-file:imports`. The descriptor arm returns the candidate set without setting `module_certain` (`cross_file.rs:5232-5241`), so `CertainCtx`/`OwnerFacts` never lift it.
- **low**: would be `Via::Descriptor` edges (`descriptor.rs:141`), but those never occur for GraphQL (see 09).
- **Mermaid `?` marker**: present on every cross-file edge (fixture `t5`: `N4 -->|?| N0`).

## 15. Dynamic use & reflection

- Not applicable. `dynuse` covers Python, JS, Java, C#, Ruby and PHP only (CLAUDE.md, dispatch section).
- GraphQL has no reflection. Introspection queries are executable documents, which are not extracted.

## 16. Known gaps & limitations

- **Descriptor link is structurally dead for GraphQL**: there are no field nodes and no owners, so `link_descriptors` never produces an SDL → resolver edge (`descriptor.rs:105-107`; fixture `t2`). This contradicts `graph.rs:86-88` ("a GraphQL field to its resolver") and README:1005.
- **Fields are not callables**: the resolver-facing unit (`Query.city`) does not exist in the graph, so per-field liveness and resolver mapping are impossible.
- **Type-system extensions are ignored**: references inside `extend type/interface/union/input/enum/schema` are lost (fixture `t1`: no `City -> Person`). This matters for federated and modular schemas, which use `extend` heavily.
- **`schema { ... }` root operation types are not references**, and directive definitions and applications are not modelled (fixture `t1`).
- **Executable documents are fully ignored**: queries, mutations, subscriptions, fragments and variable types (fixture `t1`: `ops.graphql` gives 0 callables, 0 references). There is no link from client operations to the schema.
- **Embedded SDL is not seen**: SDL in JS/TS `gql` template literals or `typeDefs` strings is not extracted (no code path). The `graphql-yoga` and `apollo-server` rule gaps say so (`rules.rs:8492`, `8538`).
- **Global by-name cross-file match with a hard-coded ≤4 cap**:
  - over 4 candidates is refused as `candidates-in-other-files`;
  - unrelated schemas sharing a type name cross-link;
  - `--fanout-cap` has no effect (fixture `t5`).
- **No error-recovery hardening**: a missing `}` swallows the next type definition and misattributes its references (fixture `t3`). The legacy `implements A, B` comma syntax drops every interface after the first (fixture `t4`).
- **Duplicate type names**: the same type defined in several files gives several identically named nodes with no file qualifier (fixture `t1`: `Ext` ×2).
- **`--report-unreferenced` lists descriptor nodes** that `--dead-code` withholds (fixture `t1`).
- **License metadata**: `tree-sitter-graphql` declares no `license` field and ships a plain MIT `LICENSE` file (README:1551-1553).
- **Tests**: four inline unit tests (`graphql.rs:187`, `193`, `214`, `225`). No integration fixture under `crates/*/tests` is GraphQL-specific (grep finds only the `detect_prefixes.rs` exclusion), and no ignored tests. The benchmark entry is `graphql-github` (octokit/graphql-schema, `scripts/benchmark.sh:104`; README:1356 shows 1,623 nodes / 5,722 edges).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.graphql`/`.gql`/`.graphqls` (case-insensitive); no embedded/JSON detection; no error-recovery hardening; legacy comma `implements` breaks the parse |
| 02. Callables extracted | partial | 6 named type kinds only; no fields, directives, schema block, extensions or operations/fragments |
| 03. Call-site forms | partial | field/arg/input/implements/union type references; no directive, default-value, root-operation, extension or executable-document refs |
| 04. Imports & module resolution | minimal | no import forms; global by-name ≤4 cap, all medium; `--fanout-cap` ignored |
| 05. Type inference & receiver typing | n/a | references are type names; no receivers |
| 06. Inheritance, dispatch & overloading | minimal | `implements`/union as forward refs only; no base records or dyn fan-out |
| 07. Scoping & shadowing | n/a | no scopes; same-file binding wins first |
| 08. Macros, preprocessor & codegen | n/a | no macros; directives not modelled |
| 09. FFI & cross-language links | none | listed as a descriptor language, but the owner-less qns mean no SDL → resolver edge is ever produced |
| 10. Frameworks & entry points | none | no graphql-language rules; host-side async-graphql/juniper (rust) and gap-only yoga/apollo (js/ts) |
| 11. Test harness entries | n/a | no test constructs |
| 12. Dead-code analysis | minimal | descriptor class: every finding withheld; `--report-unreferenced` still lists nodes |
| 13. Builtins & stdlib tables | minimal | no stdlib file; 5 built-in scalars filtered in the plugin |
| 14. Confidence & certainty | partial | intra-file high; every cross-file match medium (`?`), even single candidates |
| 15. Dynamic use & reflection | n/a | no reflection in SDL |
| 16. Known gaps & limitations | n/a | see list: dead descriptor link, no fields, extensions ignored, executable docs ignored |
