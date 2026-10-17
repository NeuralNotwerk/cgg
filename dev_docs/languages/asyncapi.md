# asyncapi

The AsyncAPI plugin (`AsyncApiPlugin`, `crates/cgg-lang/src/plugins/asyncapi.rs`, 170 lines, registered at `crates/cgg-lang/src/plugins.rs:168`) is one of the five interface/descriptor languages. It maps an event/message API document onto the call-graph model: every named channel, operation (AsyncAPI 3), component message, component schema and component parameter becomes a callable node, and every `$ref` pointer becomes a reference from the shape that contains it to the shape the pointer's last segment names. It has no grammar of its own. It parses with the published `tree-sitter-yaml` crate (`tree_sitter_yaml::LANGUAGE`, `asyncapi.rs:36-38`), which also reads JSON because JSON is flow-style YAML. All tree walking lives in the shared helper module `crates/cgg-lang/src/plugins/structured.rs`, which the OpenAPI plugin uses too. The `LanguagePlugin` trait has no `resolver_kind` (grep finds none in `crates/`). Resolution is generic: byte-span containment in `intra_file`, then a global match on simple name in `cross_file` that allows at most 4 candidates (`crates/cgg-resolve/src/cross_file.rs:5221-5240`). Version tested: cgg 0.11.0 (`target/release/cgg`). All fixtures are under `<scratch>/parity/asyncapi/`.

## 01. Detection & parsing

Supported:

- **Extensions `.yaml` / `.yml` / `.json`**, matched without regard to case. Evidence: `asyncapi.rs:30-32`, `detect.rs:88-89`. In fixture `detect/upper.YAML`, a `.YAML` file was detected as `content:asyncapi`.
- **Content detection is required.** For these extensions the detector reads the first 8 KiB (`detect.rs:157-159`) and looks for a root `asyncapi:` key. With no match the file is `Unknown`. It never falls through to the extension rule, so ordinary YAML/JSON is left alone (`detect.rs:78-95`). `detected_via = "content:asyncapi"`.

  ```yaml
  asyncapi: 2.6.0
  info: {title: x}
  ```

  Evidence: test `detect.rs::asyncapi_detects_by_content` (`detect.rs:819`), test `ordinary_yaml_json_stays_unknown`, and fixture `detect/plain.yaml`, which was skipped as `unknown-extension`.
- **Two matchers** (`detect.rs:173-200`). The first matches a line-leading `key:`, quoted or not, in the first 64 lines (YAML and pretty-printed JSON). The second matches a quoted `"asyncapi"` in the first 2048 bytes of a document that starts with `{` (minified JSON). That 2048-byte window is cut back to a char boundary (test `a_non_ascii_json_head_does_not_panic`). In fixture `json/min.json`, minified JSON with `asyncapi` as the second key was detected.
- **A leading comment is fine.** Fixture `v2/api.yml` starts with `# comment first` and was detected.
- **Shebangs: none** (`asyncapi.rs:33-35`). The shebang rule runs before content sniffing (`detect.rs:62-74`).
- **Parsing.** One grammar, `tree-sitter-yaml`, for both YAML and JSON. There is no `ts_language_for` override, so the trait default applies (`lib.rs:322`). `structured::unwrap` (`structured.rs:18-34`) looks through `flow_node`/`block_node` and skips `anchor`/`tag`/`comment`. Block and flow mappings and pairs are both handled (`structured.rs:50-56`). In fixture `json/api.json`, JSON gave the same defs and edges as YAML.
- **Depth guard.** The generic `MAX_TREE_DEPTH = 4000` (`crates/cgg/src/lib.rs:155`, checked at `lib.rs:451`) applies. CHANGELOG (around line 2547) lists "the YAML/JSON descriptor languages" among those that overflowed the stack before 0.8.5. A deeper file is skipped as `TooDeep`.
- **Long-line skip.** `max_line_bytes` is not overridden, so the default `None` applies (no LongLine skip, `lib.rs:309-315`). The `--skip-minified` heuristic covers `.js/.mjs/.cjs/.css` only (`crates/cgg-walk/src/lib.rs:19-22`), so minified AsyncAPI JSON is always analyzed.
- **Locals.** `locals::collect` returns nothing for asyncapi (`locals.rs:392`, `_ => return None`).

Not supported / partial:

- **False-positive detection.** The line matcher calls `trim_start()` (`detect.rs:175`), so an indented, non-root `asyncapi:` key anywhere in the first 64 lines also claims the file. In fixture `detect/nested.yaml` (`config:\n  asyncapi: true\nchannels: ...`), the file was detected as `content:asyncapi` and extracted a channel `c`. The comment there says "root key", but the code does not check the root.
- **First key wins between OpenAPI and AsyncAPI.** Fixture `detect/both.yaml` (`openapi:` then `asyncapi:`) was detected as `content:openapi`.
- **Multi-document YAML: only the first document is read.** `document_root` takes the first `document` node (`structured.rs:94-101`). In fixture `detect/multidoc.yaml`, only channel `c1` was extracted and `c2` was silently dropped.
- **A root key past line 64 or byte 8192** (huge leading comments or license headers) is not detected. Unverified by fixture; the limits come from the code at `detect.rs:157-159, 174`.

## 02. Callables extracted

Every definition is a `DefRecord` with `variant: DefVariant::FreeFunction` (`structured.rs:121-133`). In JSON output it is `"kind": "function"` (fixture `v3.json`). `qualified_name == simple_name ==` the map key, so there is no owner or qualification (`structured.rs:149`). The span is the value node's byte range. `signature_hint = "<kind> <name>"`. Visibility is empty and attributes are empty.

Supported:

- **`components/schemas/<Name>`** has kind `schema` (`asyncapi.rs:55`).
- **`components/messages/<Name>`** has kind `message` (`asyncapi.rs:56`).
- **`components/parameters/<Name>`** has kind `parameter` (`asyncapi.rs:57`).
- **`channels/<name>`** has kind `channel`, for both AsyncAPI 2 and 3. Names may contain `/` (`light/measured`) (`asyncapi.rs:64`).
- **`operations/<name>`** has kind `operation`, AsyncAPI 3 only (`asyncapi.rs:65-71`).

  ```yaml
  channels:
    light/measured:
      publish: {message: {$ref: '#/components/messages/LightMeasured'}}
  components:
    messages: {LightMeasured: {payload: {$ref: '#/components/schemas/P'}}}
    schemas: {P: {type: object}}
  ```

  Evidence: tests `asyncapi.rs::extracts_channel_message_schema_defs` and `channel_ref_falls_inside_channel_def`. Fixture `v3/api.yaml` gave 9 defs and fixture `v2/api.yml` gave 6.
- **Order of extraction**: schemas, messages, parameters, channels, operations (`asyncapi.rs:54-71`).

Not supported:

- **AsyncAPI 2 `operationId`** inside `publish`/`subscribe` is not a callable. In fixture `v2/api.yml`, `receiveLight` and `sendLight` produced no node, and the operation's `$ref`s attach to the channel (`asyncapi.rs:14-15`).
- **Channel-local messages** (AsyncAPI 3 `channels.X.messages.Y`) are not nodes. Only their `$ref` is seen, and it is attributed to the channel.
- **Other `components/*` sections are not extracted**: `channels`, `operations`, `servers`, `serverVariables`, `securitySchemes`, `correlationIds`, `operationTraits`, `messageTraits`, `replies`, `replyAddresses`, `externalDocs`, `tags`, `*Bindings`. In fixture `v3/api.yaml`, `$ref`s to `operationTraits/kafka`, `messageTraits/common`, `securitySchemes/saslScram` and `replies/ack` all ended up as `no-candidate-in-file` external calls. `components/channels/reusable` and `components/operations/reusableOp` gave no node.
- **Inline (anonymous) schemas and properties** are not nodes. Their `$ref`s attach to the enclosing named shape.
- **Same-name shapes of different kinds are not distinguished.** Message `UserSignedUp` and schema `UserSignedUp` are two nodes with identical qualified names that differ only in `signature_hint`. Mermaid renders both as `["UserSignedUp"]` (fixture `v3.mmd`, N0 and N2).
- **No servers or info node.** `$ref`s under `servers` have no enclosing callable (`src: null` in fixture `v3` for `saslScram`).

## 03. Call-site forms

The only site form is a `$ref` pair (`structured.rs:154-181`). `collect_refs` recurses over the whole tree. For each pair whose key is `$ref`, it records a `RefRecord` with `name` set to the last `/` segment of the pointer (`ref_simple_name`, `structured.rs:105-107`), an empty `receiver_hint`, and `site_byte` at the value's start. Whatever def's span contains that site is the caller (`asyncapi.rs:11-15`).

Supported:

- **Local pointer** `$ref: '#/components/messages/M'` gives the reference `M`. Evidence: test `channel_to_message_and_message_to_schema_refs`. Fixture `v2`: `channel light/measured -> message lightMeasured`, high, intra-file.
- **Single quotes, double quotes, unquoted and flow (`{$ref: ...}`)** all work (`scalar_text`, `structured.rs:37-48`). Fixture `v2/api.yml` uses all three.
- **`$ref`s inside `oneOf`/`allOf`/sequences** work. In fixture `v2`, the publish `oneOf` gave edges to both `lightMeasured` and `turnOn`. In fixture `v3`, `allOf` gave `schema UserSignedUp -> schema User`.
- **Self-reference** (recursive schema) gives a self edge. Fixture `v3`: `schema User -> schema User`, high.
- **Operation → channel** (AsyncAPI 3 `channel: {$ref: '#/channels/X'}`). Fixture `v3`: `operation onUserSignUp -> channel userSignedUp`, high.
- **Repeated `$ref` to the same target from one shape** gives one edge per site in JSON output. Fixture `v2.json` has `light/measured -> lightMeasured` twice.

Not applicable: bare, method, qualified, chained, constructor, generic, operator, paren-less and value-ref call forms. There are no receivers, no arity (`RefRecord::arity` is never set) and no registrar arguments. The plugin ignores `ExtractCtx` (`_ctx`, `asyncapi.rs:42`), so `--reference-edges` and registrar verbs have no effect. With `--dynamic-dispatch --reference-edges`, the mermaid output of fixture `v3` was 25 lines, the same as the default.

Not supported / wrong:

- **The file part of a pointer is discarded.** `x.yaml#/components/messages/M` and `common.yaml#/Foo` keep only the last segment (`structured.rs:103-107`). In fixture `fan/main.yaml`, `x.yaml#/.../M` bound at **high** to the local `M`, even though the pointer names another file.
- **Deep pointers resolve to the wrong node.** `#/components/schemas/B/properties/name` gives the name `name`. In fixture `deep/d.yaml`, the result was `A -> name` (the unrelated root schema `name`) instead of `A -> B`.
- **JSON-pointer escapes are not decoded** (`~1` for `/`, `~0` for `~`). In fixture `v3`, `$ref: '#/channels/user~1deleted'` was left unresolved (`no-candidate-in-file`, name `user~1deleted`) even though channel `user/deleted` exists.
- **A pointer to a whole document or URL** (`https://example.com/schemas/Ext.json`) gives the name `Ext.json`, which is unresolved (fixture `v3`).
- **A channel-message pointer** (`#/channels/userSignedUp/messages/UserSignedUp`) is matched by its last segment against every same-named shape. In fixture `v3`, it bound at medium to both message `UserSignedUp` and schema `UserSignedUp`.
- **YAML aliases (`*anchor`) are not followed.** In fixture `alias/a.yaml`, `message: *mm` in a channel and `N: *mm` produced no edge from the channel or from `N`. Only the anchored original `M -> S` was linked.

## 04. Imports & module resolution

- **No import statements.** Cross-file linking is a global simple-name lookup over all asyncapi callables (`cross_file.rs:5221-5240`). It binds every candidate when 1 to 4 exist. More than 4 gives `candidates-in-other-files`. The cap is hard-coded and ignores `--fanout-cap`. In fixture `fan` (5 files each declaring `Shared`), the result was unresolved with `{"stage":"candidates-in-other-files","detail":{"candidates":5}}`, and the same with `--fanout-cap 10`.
- **Cross-file edges are `medium`** with resolver `cross-file:imports`. In fixture `v3`, `schema User -> schema Address` (in `common.yaml`) was medium.
- **The same file takes precedence.** In fixture `json`, both files declare `M`, and each channel bound only its own file's `M`, intra-file at high.
- **Not applicable**: `FileFacts::packages`, manifests, `heads.rs` external-module refusal, `modules.rs` `not-in-imported-module`, and workspace vocab. None of these name asyncapi (grep).
- **Gap**: the `$ref` file path, which is the actual "import", is never used to narrow candidates (section 03).
- **Gap**: languages are kept separate. The lookup key is `(lang, name)` (`cross_file.rs:5233`), so an AsyncAPI `$ref` never reaches an OpenAPI schema of the same name (inferred from the key; not fixture-tested).

## 05. Type inference & receiver typing

Not applicable. There are no receivers, variables or types. `type_hints` has no asyncapi branch (grep). The only mention is a perf comment noting that `asyncapi-spec` was +82.6% slower under one map strategy (`type_hints.rs:500`).

## 06. Inheritance, dispatch & overloading

- **Not applicable / none.** There are no class records, bases or `super`. asyncapi is not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS` (grep finds no asyncapi in `dispatch.rs` or `type_hints.rs`). `--dynamic-dispatch` does not change the graph (fixture `v3`, 25 lines either way).
- **`allOf` composition** is the nearest analogue to inheritance. It is treated as a plain `$ref` edge from the composing schema (fixture `v3`, `UserSignedUp -> User`). There is no modelling of inherited properties. AsyncAPI 2 message `traits` would be the nearest analogue to mixins, but trait components are not nodes (section 02).

## 07. Scoping & shadowing

- **Byte containment is the only scoping.** A `$ref` belongs to the named shape whose value span contains it (`structured.rs:5-11`, `push_def` spans at `structured.rs:125-128`).
- **Nesting**: components are separate top-level maps, so def spans do not nest. A channel's AsyncAPI 2 `publish`/`subscribe` blocks lie inside the channel span (`asyncapi.rs:14-15`; test `channel_ref_falls_inside_channel_def`).
- **Not applicable**: `FileFacts::locals` (`locals.rs:392` returns `None`), shadowed-by-local, lexical narrowing, positional shadowing, conditional definitions.
- **`$ref`s outside every def** (under `servers`, `info`, or a non-extracted component) have `src: null` and are counted in the "(unattributed)" bucket (fixture `v3` audit: `saslScram`).

## 08. Macros, preprocessor & codegen

Not applicable. AsyncAPI has no macros. YAML anchors and aliases (`&a` / `*a`) are the nearest equivalent. `unwrap` skips `anchor`/`tag` nodes (`structured.rs:24`), but aliases are never expanded (fixture `alias`). Nothing is recorded in `name_mentions`, and there is no `macro:<name>` tagging.

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: not applicable (no asyncapi in `ffi.rs`).
- **Descriptor → implementation linking: not covered.** `DESCRIPTOR_LANGUAGES = ["proto", "graphql", "smithy"]` (`crates/cgg-resolve/src/descriptor.rs:47`) leaves out asyncapi (and openapi). An AsyncAPI operation (`onUserSignUp`) or a v2 `operationId` is never linked to the host-language handler that implements it, and no `Via::Descriptor` edge is produced. This is despite the module comment at `descriptor.rs:3` and README (around line 1005) listing AsyncAPI among the descriptor languages for "Descriptor linking". In fact the README step means only the `$ref` edges.
- **Markup code-behind**: not applicable.

## 10. Frameworks & entry points

- **No framework rule targets asyncapi.** `grep 'language: "' crates/cgg-core/src/frameworks/rules.rs` lists 36 languages, and neither asyncapi nor openapi is among them. Every rule is matched on `rule.language` (`crates/cgg-resolve/src/frameworks/mod.rs:648, 709`), so no rule can apply.
- **No rule matches channels, operations or messages.** There are no rule ids, registrars, member_sources, signature_methods, container_attributes, markup_kinds or visibility entries for asyncapi.
- **Related gap prose**: the Connexion `gap` (`rules.rs:8163`) covers Python handlers named only by an OpenAPI `operationId` string. It is OpenAPI, not AsyncAPI, and no rule reads AsyncAPI `operationId` or `x-` handler keys to root host-language code.
- **Benchmark**: `asyncapi-spec` (`https://github.com/asyncapi/spec.git`, subdir `examples`) is in `scripts/benchmark.sh:106`, and its APPS line is at `benchmark.sh:348`. README benchmark row `README.md:1358`: 279 callables, 374 edges, 7%, 15 ms.

## 11. Test harness entries

None. There are no rules of kind `test` for asyncapi, and AsyncAPI has no test-file concept. `TEST_CLASS_MEMBERS_ARE_ROOTS` does not apply.

## 12. Dead-code analysis

- **The class is `Descriptor`** (`crates/cgg-resolve/src/deadcode/caps.rs:41`, `DESCRIPTOR`, and its use at `caps.rs:163-164`). Every asyncapi finding is **withheld** with `SuppressionReason::DescriptorLanguage` (`deadcode/mod.rs:807-810`). The reasoning: an unreferenced schema is a wire contract whose consumers live elsewhere. `Evidence::LanguageIsDescriptor` lowers confidence (`deadcode/mod.rs:926`). Test `descriptor_language_findings_are_withheld_with_a_count` (`deadcode/mod.rs:2017`) uses openapi; asyncapi takes the same path through the `DESCRIPTOR` list.
- **Fixture `alias`** with `--dead-code` reported 0 findings, "4 withheld (descriptor language)" for never-referenced, and "1 withheld" for reachable-only-from-dead-code. Roots: 0, `root-reachability disabled:no-roots-found`.
- **Signal coverage row**: `asyncapi ... class descriptor, vis no, attrs no, refs no, disp no, reach 0%`. The plugin does not override `signals()`, so every `PluginSignals` field is `false` (`lib.rs:304-306`).
- **No root kinds**: no public/exported roots, no visibility, no confidence caps, no unresolved-doubt handling specific to asyncapi.
- **No `BLIND_SPOTS` entry** (`caps.rs:44-71`).

## 13. Builtins & stdlib tables

- **No `crates/cgg-core/src/stdlib/asyncapi.txt`.** None of the 30 stdlib files and none of the `include_str!` entries in `stdlib.rs:184-213` is for asyncapi. No `rust_types`-like tables, and no `CAST_INTRINSICS`/`CONSTRUCTING_METHODS` entries.
- **`--include-stdlib` mints nothing** for asyncapi. **`--include-external`** turns each unresolved `$ref` into an `<external>::<name>` leaf. In fixture `v3`: `<external>::kafka`, `<external>::user~1deleted`, `<external>::ack`, `<external>::common`, `<external>::Ext.json`, with `ext` edges. The unattributed `saslScram` gets no node because it has no source callable.
- **Built-in AsyncAPI keywords** are not names, so no builtin filter is needed. Only `$ref` values become references.

## 14. Confidence & certainty

- **`high`**: a same-file `$ref` with exactly one same-named def, from resolver `intra-file`. Fixture `v3` has 7 high edges and fixture `v2` has all 6 high. This includes the wrong `x.yaml#/.../M` and deep-pointer `A -> name` cases (section 03), so `high` here means "one candidate in this file", not "structurally certain".
- **`medium`**: every cross-file bind, and every same-file bind where several same-named shapes exist (message and schema both named `UserSignedUp`). These fall through to `cross-file:imports` (fixture `v3`: 6 medium edges, each rendered with the mermaid `?` label).
- **`low`**: never produced. Descriptor-link low edges do not apply (section 09).
- **`CertainCtx`/`OwnerFacts`**: not applicable (no receivers).

## 15. Dynamic use & reflection

Not applicable. There is no `dynuse` coverage, and AsyncAPI has no eval, reflection or module loading. Runtime `$ref` dereferencing by tools is the only "dynamic" behaviour, and it is approximated statically (section 03).

## 16. Known gaps & limitations

Gaps found while cataloguing (all fixture-verified unless marked):

- Detection accepts a non-root, indented `asyncapi:` key in the first 64 lines (`detect/nested.yaml`).
- Only the first YAML document of a multi-document stream is extracted (`detect/multidoc.yaml`, `structured.rs:96-98`).
- The file part of a `$ref` is discarded, so a cross-file pointer can bind at high to a same-named local shape (`fan/main.yaml`).
- Deep JSON pointers take their last segment (a property name) as the target (`deep/d.yaml`).
- `~1`/`~0` JSON-pointer escapes are not decoded, so channel refs with `/` in the name never resolve (`v3`, `user~1deleted`).
- YAML aliases are not followed (`alias/a.yaml`).
- Same-named shapes of different kinds (message and schema) collide. They produce medium fan-out to both and identical mermaid labels (`v3`).
- AsyncAPI 2 `operationId`, channel-local messages, and these component sections are not callables: `components/channels`, `operations`, `messageTraits`, `operationTraits`, `securitySchemes`, `servers`, `replies`, `correlationIds`, bindings.
- No descriptor → implementation link. asyncapi is missing from `DESCRIPTOR_LANGUAGES` (`descriptor.rs:47`), so handlers serving an AsyncAPI operation look dead in the host language.
- The cross-file candidate cap is a fixed 4 that ignores `--fanout-cap` (`cross_file.rs:5235`; fixture `fan`).
- There is no qualified name or owner, so `kind` is visible only through `signature_hint`, and nodes are all `function`.
- No `gap_for()` text, no CHANGELOG follow-up items and no ignored tests for asyncapi were found. The plugin has 4 unit tests (`asyncapi.rs:119-169`) plus 1 detection test (`detect.rs:819`). No integration fixture under `crates/*/tests` names asyncapi; `crates/cgg/tests/detect_prefixes.rs:94,179` only skips it.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | Content-sniffed YAML/JSON with tree-sitter-yaml; a nested `asyncapi:` key causes false positives; multi-doc reads only the first document |
| 02 Callables extracted | partial | schemas/messages/parameters/channels/operations; no v2 operationId, traits, servers, component channels/ops; no owner |
| 03 Call-site forms | partial | `$ref` only, by last pointer segment; file part, `~1` escapes, deep pointers and aliases mishandled |
| 04 Imports & module resolution | minimal | Global name match across files with ≤4 candidates (medium); `$ref` file path ignored; cap ignores `--fanout-cap` |
| 05 Type inference & receiver typing | n/a | No receivers or types |
| 06 Inheritance, dispatch & overloading | n/a | `allOf` is a plain edge; not in any dispatch or overload list |
| 07 Scoping & shadowing | minimal | Byte containment only; refs outside any def are unattributed |
| 08 Macros, preprocessor & codegen | none | YAML anchors/aliases not expanded |
| 09 FFI & cross-language links | none | Not in `DESCRIPTOR_LANGUAGES`; no operation → handler link |
| 10 Frameworks & entry points | none | No rules for asyncapi |
| 11 Test harness entries | n/a | No test concept |
| 12 Dead-code analysis | minimal | Descriptor class; every finding withheld; all signals "no" |
| 13 Builtins & stdlib tables | n/a | No stdlib file; `--include-external` mints `<external>::` leaves |
| 14 Confidence & certainty | partial | High = unique same-file name (can be wrong); cross-file/ambiguous = medium with `?` |
| 15 Dynamic use & reflection | n/a | Not applicable |
| 16 Known gaps & limitations | n/a | 11 gaps listed above; 4 plugin unit tests + 1 detect test only |
