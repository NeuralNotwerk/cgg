# proto

The Protocol Buffers plugin (`ProtoPlugin`) lives in `crates/cgg-lang/src/plugins/proto.rs`, which is 294 lines including tests. Its job is descriptor topology: it maps a `.proto` schema onto cgg's callable/edge model (`proto.rs:1-17`).

- **Grammar**: the crates.io crate `tree-sitter-proto = "0.4"` (`Cargo.toml:125`, `crates/cgg-lang/Cargo.toml:62`), bound as `tree_sitter_proto::LANGUAGE` (`proto.rs:37-39`). Nothing is vendored.
- **Registration**: `crates/cgg-lang/src/plugins.rs:165` (`reg.register(Box::new(proto::ProtoPlugin))`; module declared at `plugins.rs:39`).
- **Resolver kind**: the `LanguagePlugin` trait has no `resolver_kind` method, so no per-plugin resolver kind exists. Proto is one of the *descriptor languages*. It has a dedicated branch in cross-file resolution, a global by-simple-name match capped at 4 candidates (`crates/cgg-resolve/src/cross_file.rs:5222-5241`). It also has a dedicated post-pass that links an rpc to the host-language method implementing it (`crates/cgg-resolve/src/descriptor.rs`, `DESCRIPTOR_LANGUAGES = ["proto","graphql","smithy"]` at `:47`, family `grpc` at `:52`).
- **Dead code**: the report classifies proto as `LanguageClass::Descriptor` (`crates/cgg-resolve/src/deadcode/caps.rs:41,163`) and withholds every proto finding.
- **Trait defaults**: the plugin overrides none of `signals()`, `max_line_bytes()` or `ts_language_for()`, so it inherits the defaults at `crates/cgg-lang/src/lib.rs:304,313,322`.
- **Extraction context**: `ExtractCtx` is ignored (`_ctx`, `proto.rs:43`).

All fixtures below live under `<scratch>/parity/proto/{a,b,c}` and were run with `target/release/cgg` 0.11.0.

## 01. Detection & parsing

- **Extension**: `.proto` only (`proto.rs:31-33`). Fixture `a` reported `detected_via: "extension:.proto"` for every `.proto` file.
- **Shebangs**: none (`proto.rs:34-36`).
- **Content detection**: none. `grep proto crates/cgg-lang/src/detect.rs` finds nothing; the YAML/JSON content rule there serves OpenAPI/AsyncAPI only.
- **Grammar**: one grammar, `tree_sitter_proto::LANGUAGE`, with no per-file dialect choice.
  - `syntax = "proto3"` parses `ok` (fixture `a`).
  - `syntax = "proto2"` with `optional group Result = 1 {…}`, `extensions 100 to 199;` and `reserved` gave `parse_status: "error"` (fixture `b/p2.proto`). Error recovery cut `message Outer` short: it was recorded as lines 2-3 rather than 2-6. The token `group` leaked through as a reference name and was screened `external` (audit `external_calls` entry `"name": "group"`).
  - `edition = "2023";` parses `ok` (fixture `b/ed.proto`).
- **Re-parse / fallback**: none. On a syntax error, whatever tree-sitter recovered is walked as-is. In fixture `b/broken.proto`, `message Ok` was extracted and the truncated `message Bad { Leaf l =` was lost.
- **Skip conditions** are the generic ones only:
  - Depth cap: `MAX_TREE_DEPTH` 4,000 gives `SkipReason::TooDeep` (generic, `crates/cgg/src/lib.rs`).
  - No `max_line_bytes` override, so there is no `LongLine` skip.
  - There is no minified handling, which is irrelevant since there are no locals.
- **Detect-prefix test**: proto is exempted because "an import is a schema reference" (`crates/cgg/tests/detect_prefixes.rs:93-96,177-182`).

## 02. Callables extracted

- **`message Name {…}` → `DefVariant::FreeFunction`**:
  - JSON kind is `function`, with signature hint `message Name` (`proto.rs:89-92,167-198`).
  - The qualified name is `package.Name` when a `package` statement exists, else `Name` (`proto.rs:177-181`).
  - Example: `package demo.v1; message HelloRequest {}` gives `demo.v1.HelloRequest` (fixture `a`).
  - Test: `extracts_package_qualified_defs` (`proto.rs:271-281`).
- **`enum Name {…}` → `FreeFunction`**, with hint `enum Name` (`proto.rs:93-96`). For example `enum Color { RED = 0; }` gives `demo.v1.Color` (fixture `a`, test above). Enum *values* are not callables.
- **`service Name {…}` → `FreeFunction`**, with hint `service Name` (`proto.rs:97-107`). For example `demo.v1.Greeter` (fixture `a`).
- **`rpc M (Req) returns (Resp);` → `DefVariant::InherentMethod`**:
  - JSON kind is `method` and the qualified name is `package.Service.M`, hint `rpc M` (`proto.rs:131-165`).
  - Streaming rpcs (`rpc S (stream A) returns (stream B) { option … }`) are extracted the same way, as `demo.v1.Greeter.StreamHellos` spanning lines 30-32 (fixture `a`).
  - When there is no package the name is `Service.M`, e.g. `NoPkg.Do` (fixture `b`).
- **Package**: only the first top-level `package` statement's `full_ident` is read (`find_package`, `proto.rs:61-74`).
- **NOT extracted**:
  - **Nested messages and enums.** `walk` returns after recording a top-level message/enum without descending into it (`proto.rs:89-96`). `message HelloRequest { message Inner {…} enum Kind {…} }` yields no `Inner` or `Kind` node (fixture `a`: 11 proto callables, none of them nested). References to them go unresolved; see 16.
  - `extend Foo { … }` blocks: no node (fixture `a`, `b`).
  - Fields, `oneof`, `map`, `option`, `reserved`, `extensions` and enum values.
  - Proto2 `group` (fixture `b`).
  - Service-level and rpc-level `option`s.
- **Visibility and attributes** are always empty (`proto.rs:149-150,195-196`).
- **Overloads and clauses**: not applicable. Protobuf has no overloading.

## 03. Call-site forms

Proto has no calls. Every "reference" the plugin records is a type reference, built as a `RefRecord` with only `name`, `site_line` and `site_byte` set (`proto.rs:205-230`).

- **Field type naming a message/enum**: the grammar emits a `message_or_enum_type` node only for non-scalar types (`proto.rs:15-17`). For example `Common common = 2;` records a reference `Common`. Scalars (`string`, `int32`) never produce one (test `message_field_and_rpc_references`, `proto.rs:284-293`).
- **Dotted or fully-qualified type**: only the last segment is kept (`raw.rsplit('.')`, `proto.rs:208-209`). `.demo.v1.Meta` becomes `Meta`, and `google.protobuf.Timestamp` becomes `Timestamp`. The package qualifier is thrown away (fixture `a`; consequences in 04 and 16).
- **`repeated T`, `map<K, V>` value type, and `oneof` member types**: all recorded.
  - Fixture `a` produced `HelloRequest → Item` three times, from site lines 11 (map value), 13 (oneof member) and 16 (inside nested `Inner`).
  - Fixture `a` also produced `HelloRequest → Color` from line 14 (oneof).
- **rpc request/response types, with or without `stream`**: recorded from inside the service body.
  - **Caller attribution**: the site lies inside the rpc's span, so the edge's source is the rpc node (`Greeter.SayHello → HelloRequest`, `→ HelloReply`), not the service.
  - The `service` node itself got **no** outgoing edges (fixture `a`, JSON edges list).
  - The README row (`README.md:527`) still says "`service` rpc → request/response message edges".
- **Types inside a nested message** are attributed to the enclosing *top-level* message, because the nested message has no node of its own. The line-16 `Item it` inside `Inner` became `HelloRequest → Item` (fixture `a`).
- **Not recorded**:
  - Types inside `extend` blocks: `extend Item { Meta extra = 100; }` gave no edge, because `walk` collects references only beneath message/enum/service nodes (fixture `a`, `b`).
  - Types named in `option (…)` values.
- **Not applicable**: receivers, arity, generics, value refs, registrar arguments, paren-less calls, method refs. `RefRecord` defaults are used and none of those fields is set.

## 04. Imports & module resolution

- **`import "x.proto"` and `import public "x.proto"`**: **not read**. The plugin never looks at `import` nodes (no `import` handling in `proto.rs`), and `FileFacts::imports` stays empty.
- **Cross-file binding** is a global by-simple-name match within the language, capped at 4 candidates (`cross_file.rs:5222-5241`; also CHANGELOG "Cross-file resolution for descriptor languages", `CHANGELOG.md:5964-5967`).
  - The edges are tagged resolver `cross-file:imports` (fixture `a`) even though no import was consulted.
  - With 5 same-named messages in 5 packages, `Dup` was left unresolved with reason `candidates-in-other-files {candidates: 5}` (fixture `b`, `usesdup.proto`).
- **No package narrowing**:
  - In fixture `a`, `package demo.v1; message Common { Item item = 1; }` bound to **both** `demo.v1.Item` and `other.Item` at `medium`.
  - In fixture `c`, a nested `a.Outer.Inner` reference bound to the unrelated `b.Inner` (a wrong edge).
- **`FileFacts::packages`**: not populated. The package is only used as a qualified-name prefix.
- **`heads.rs` external-module refusal / `WorkspaceVocab`**: not applicable. Proto is not among the languages handled there (`grep proto crates/cgg-resolve/src/heads.rs` is empty).
- **`modules.rs` (`not-in-imported-module`)**: not applicable (no proto entry).
- **Unresolvable names** (well-known types such as `google.protobuf.Timestamp` or `Any`, and nested types) come out as `no-candidate-in-file` with `name_screen_applied: "external"`. They are counted as `external`, not `unresolved`: fixture `a` shows `external: 3`, `unresolved: 0` for proto in `by_language`.
- **`--include-external`** mints `<external>::Any` exit nodes for them (fixture `c`).
- **Manifest reading**: none. `buf.yaml` and `buf.work.yaml` are not read.

## 05. Type inference & receiver typing

- Not applicable: proto has no expressions, variables or receivers. `type_hints.rs` has no proto handling (`grep proto` is empty). No `typed_receiver`, `local_types` or `scoped_types` are produced.

## 06. Inheritance, dispatch & overloading

- No base-class records or `ClassDecl`s: protobuf has no inheritance. `extend` (proto2 extensions) is not modelled (02).
- Proto is not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS` (no `"proto"` string in `dispatch.rs`, `type_hints.rs` or `cross_file.rs` outside the descriptor branch).
- `--dynamic-dispatch` adds nothing for proto. Fixture `c` produced the same 19-line output with and without `--dynamic-dispatch --reference-edges --include-stdlib`.
- **Interface → implementation**: the one dispatch-like edge is the descriptor link from an rpc to its host-language implementation (09).

## 07. Scoping & shadowing

- `FileFacts::locals`: none (no proto entry in `crates/cgg-lang/src/locals.rs`).
- Shadowing and lexical narrowing: not applicable.
- **Nested scope**: protobuf type resolution is scope-relative (an inner `Inner` shadows an outer or package `Inner`). cgg models none of this.
  - Nested types are not extracted (02) and qualifiers are stripped (03).
  - As a result, a nested type reference either goes unresolved or binds to a same-named type anywhere in the tree (fixture `c`, `a.Outer → b.Inner`).
- Same-file references bind via the intra-file linker at `high` (fixture `a`: all nine same-file edges `intra-file`, `high`).

## 08. Macros, preprocessor & codegen

- Not applicable: no macros or preprocessor in protobuf.
- No `macro:<name>` tagging and no `name_mentions`.
- The generated code that `protoc` would produce (`*.pb.go`, `*_pb2.py`, `*Grpc.java`) is not synthesized. When committed, it is analyzed as ordinary host-language code.

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: no proto involvement.
- **Descriptor link from rpc to host-language implementation** (`descriptor::link_descriptors`, `crates/cgg-resolve/src/descriptor.rs:60-157`; invoked at `crates/cgg/src/lib.rs:997-1002`, after FFI).
  - **Which proto nodes link**: only `CallableKind::Method` nodes, i.e. rpcs. Messages, enums and services never link (`descriptor.rs:104-106`; the CHANGELOG fix "a `message` is not an RPC", `CHANGELOG.md:219-221`).
  - **Matching rule**: a host method links when its simple name equals the rpc name case-insensitively, **and** its owner contains the service name's last segment case-insensitively (`descriptor.rs:110-133`).
  - **Edge properties**: confidence is always `Low`, with `Via::Descriptor("grpc")`, resolver `descriptor-linker`, and mermaid label `?desc` (`descriptor.rs:134-145`; `crates/cgg-format/src/mermaid.rs:37`; dot styling at `dot.rs:24`).
  - **Supported**:
    - Go `GreeterServer.SayHello` (fixture `a`; test `a_proto_rpc_links_to_its_go_implementation`, `crates/cgg/tests/frameworks.rs:775`; unit test `rpc_links_to_a_conventionally_named_implementation`, `descriptor.rs:193`).
    - Java `GreeterImpl.sayHello`, where only the first letter's case differs (unit test `java_lowercases_the_first_letter_and_still_links`, `descriptor.rs:205`).
    - Java `NoPkgImpl.Do` (fixture `b`).
    - Python `NoPkgServicer.Do` (fixture `b`).
  - **Correctly refused**:
    - A method whose owner does not name the service, e.g. `Cache.SayHello` (fixture `a`; tests `a_bare_method_name_match_is_not_enough`, `descriptor.rs:228`, and `a_bare_method_name_does_not_link_a_descriptor`, `frameworks.rs:801`).
    - A message in a package, e.g. `a.state.Iterator` (`descriptor.rs:214`).
    - The service declaration itself (`descriptor.rs:233`).
  - **Weaknesses**:
    - The owner test is a substring check, so a service `Foo` also matches owners like `Football`.
    - There is no fan-out cap. Fixture `c` linked one rpc to six `GreeterServerN.SayHello` methods.
    - A Rust/tonic `impl Greeter for MyGreeter` matches only if the *owner* string contains `greeter` (unverified for trait-impl qualified names).
- **Markup code-behind**: not applicable.

## 10. Frameworks & entry points

- **Rule ids with `language: "proto"`**: none. `grep 'language: "proto"' crates/cgg-core/src/frameworks/rules.rs` is empty, and the per-language rule tally lists no proto.
- **Host-language gRPC rules that mention protobuf** are all *gap-only* (detect-and-explain, no entries) and apply to the host language, not proto:
  - `grpc-java` (`rules.rs:2330`)
  - `grpc-go` (`:2398`)
  - `grpc-dotnet` (`:2561`)
  - the tonic gap (`:2747`)
  - `grpc-gateway` (`:8858`)
  - Kratos (`:8887`)
  - Their gap texts note that the `option (google.api.http)` route annotations live in the `.proto` and are not read.
- **Starlark/Bazel detect list** includes `@rules_proto` and `@com_google_protobuf` (`rules.rs:6711,6721`). That is a Starlark rule.
- No member_sources, signature_methods, container_attributes, markup_kinds or registrar verbs for proto.
- `frameworks::detect` produces no `<framework-entry>` nodes for proto (fixture `a`: `framework_coverage.recognised: []`).
- **Benchmark coverage**: the benchmark lists `proto-grpc` (grpc/grpc-proto) with no framework apps (`scripts/benchmark.sh:103,387`).

## 11. Test harness entries

- None. Protobuf has no test constructs, and no `kind: test` rule names proto. `TEST_CLASS_MEMBERS_ARE_ROOTS` does not apply.

## 12. Dead-code analysis

- **Class**: proto is classified `LanguageClass::Descriptor` (`deadcode/caps.rs:41,163-164`). The stated reason is that an unreferenced node "is a wire contract whose consumers are by definition in another repository" (`caps.rs:38-40`).
- **Every finding is withheld** with `SuppressionReason::DescriptorLanguage` (`crates/cgg-resolve/src/deadcode/mod.rs:807-809`), and gets evidence `language-is-descriptor` (`mod.rs:926`; `cgg-core/src/deadcode.rs:387`).
  - Fixture `a` `--dead-code`: "proto never-referenced 4 withheld (descriptor language)" and "proto reachable-only-from-dead-code 7 withheld".
  - The signal table showed `proto … descriptor no no no no 0%`.
  - Unit test `descriptor_languages_are_classified_apart` (`caps.rs:240`, exercised with openapi).
- **Roots and signals**: no dead-code roots are produced for proto. Visibility, attributes, value refs, dispatch, exports and test tagging are all `no`, because `signals()` is the default.
- **Caps** (`implements-external-contract`, `macro-generated`, `reflection`): not applicable.

## 13. Builtins & stdlib tables

- There is no `crates/cgg-core/src/stdlib/proto.txt`, and `stdlib.rs` has no proto entry (`grep` is empty).
- Scalar types never reach resolution because the grammar separates them (03), so no builtin table is needed for them.
- **Well-known types are not modelled.** `google.protobuf.Timestamp`, `Any`, `Empty` and similar are screened `external`, not `stdlib`: fixture `a` showed `stdlib: 0`, `external: 3`.
- `--include-stdlib` adds nothing (fixture `c`). `--include-external` mints `<external>::Any` (fixture `c`).
- `CAST_INTRINSICS`, `CONSTRUCTING_METHODS` and implicit packages: not applicable.

## 14. Confidence & certainty

- **`high`**: same-file type references via `intra-file` (fixture `a`: 9 of 9 same-file edges were `high`, and mermaid showed no `?`).
- **`medium`**: every cross-file reference via the descriptor by-name branch (fixture `a`: `Common → Item` ×2 and `HelloRequest → Common`, mermaid `-->|?|`). These stay `medium` even when the package-qualified name is unambiguous, because the qualifier was stripped.
- **`low`**: every descriptor → implementation edge (`descriptor.rs:139-141`), shown in mermaid as `?desc`. The CHANGELOG records proto −288 `low` edges after the message-is-not-an-rpc fix (`CHANGELOG.md:260`).
- `CertainCtx` and `OwnerFacts` do not apply.

## 15. Dynamic use & reflection

- Not applicable: proto is not in `dynuse` coverage, and there is no reflection or module loading.
- `Any` / `google.protobuf.Any` dynamic packing is a runtime concept and is not modelled (it is screened external, 13).

## 16. Known gaps & limitations

- **Nested messages and enums are not extracted** (`proto.rs:89-96` returns without descending).
  - Their references go unresolved, e.g. `Inner` and `Kind` as `no-candidate-in-file` screened external (fixture `a`).
  - Or they bind to a same-named top-level type in another package (fixture `c`: `a.Outer → b.Inner`, a wrong edge).
- **Package and qualifier are dropped from references** (`proto.rs:208-209`). Cross-file binding ignores packages: `Item` bound to both `demo.v1.Item` and `other.Item` (fixture `a`). With 5 or more same-named types it is refused (`candidates-in-other-files`, fixture `b`).
- **`import` statements are ignored entirely**. Cross-file edges carry the label `cross-file:imports` regardless (fixture `a`).
- **`extend` blocks** produce neither a node nor references (fixture `a`, `b`).
- **Proto2 `group` fields fail to parse.** This yields `parse_status: error`, a truncated message span and a spurious `group` reference (fixture `b`).
- There is no re-parse or fallback on parse errors (fixture `b/broken.proto`).
- **The `service` node gets no edges.** All request/response edges originate at the rpc nodes, which contradicts the README row wording (`README.md:527`).
- **`option (google.api.http)` route annotations are not read**, so grpc-gateway/Kratos HTTP routes are not entries (`rules.rs:8870,8887` gap texts).
- **Descriptor linker weaknesses**:
  - It has no fan-out cap (six links in fixture `c`).
  - Its owner check is a substring match.
  - It only covers implementations that follow a naming convention, by design (`descriptor.rs:23-37`).
- **No `FileFacts::packages`, no `signals()`, and no visibility**. Dead code is fully suppressed for proto (12).
- **Test coverage is thin**:
  - 3 inline plugin tests (`proto.rs:264-293`)
  - 5 descriptor unit tests (`descriptor.rs:192-236`)
  - 2 integration tests (`frameworks.rs:775,801`)
  - No ignored tests were found (`grep -n ignore proto.rs descriptor.rs` is empty; unverified across all test crates).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.proto` only; proto3/editions parse, proto2 `group` errors; no fallback |
| 02. Callables extracted | partial | top-level message/enum/service + rpc methods; nested types, extend not extracted |
| 03. Call-site forms | partial | field/map/oneof/rpc type refs; qualifier stripped; extend/option refs missed |
| 04. Imports & module resolution | minimal | imports ignored; global simple-name match ≤4, no package narrowing |
| 05. Type inference & receiver typing | n/a | no expressions or receivers |
| 06. Inheritance, dispatch & overloading | n/a | no inheritance/overloads; extend not modelled |
| 07. Scoping & shadowing | none | nested scope resolution absent, causing wrong edges |
| 08. Macros, preprocessor & codegen | n/a | no macros; protoc output not synthesized |
| 09. FFI & cross-language links | partial | rpc → host impl by naming convention (low, uncapped, substring owner match) |
| 10. Frameworks & entry points | none | no proto rules; host gRPC rules are gap-only |
| 11. Test harness entries | n/a | no test constructs |
| 12. Dead-code analysis | none | descriptor class, every finding withheld |
| 13. Builtins & stdlib tables | none | no stdlib table; well-known types screened external |
| 14. Confidence & certainty | partial | same-file high, cross-file medium, descriptor links low |
| 15. Dynamic use & reflection | n/a | not applicable |
| 16. Known gaps & limitations | n/a | 10 concrete gaps listed above |
