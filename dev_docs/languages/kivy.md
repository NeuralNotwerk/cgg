# kivy

The Kivy plugin (`KivyPlugin`, id `"kivy"`, `crates/cgg-lang/src/plugins/kivy.rs`, 1,417 lines) reads Kivy KV markup (`.kv`). It is a markup/host-bridge plugin, not a general-purpose language plugin. Each `on_*` event-binding property (for example `on_release: root.foo()`) becomes a callable node. Every call written inside a binding, or inside another property expression, is recorded as a reference. A dedicated kv→python linker in `cgg-resolve::ffi` (`link_kivy_python`, `crates/cgg-resolve/src/ffi.rs:268`) binds those references onto **Python** methods and emits `Via::Ffi("kv-python")` edges. No crates.io crate exists for the grammar. `tree-sitter-kivy` v0.1.0 (ABI 15, MIT) is vendored under `crates/cgg-lang/vendor/kivy/` (`parser.c` + indent/dedent `scanner.c`) and compiled in `crates/cgg-lang/build.rs:29-38`. The plugin binds the raw `tree_sitter_kivy()` symbol through `LanguageFn` (`kivy.rs:24-28`). One local grammar patch accepts colon-less rule headers (`vendor/kivy/PROVENANCE.md`). There is no `resolver_kind` in the current `LanguagePlugin` trait (grep for `resolver_kind` in `crates/` finds nothing), so this item does not apply. The plugin does not override `signals()`, so every `PluginSignals` flag is false (`crates/cgg-lang/src/lib.rs:269-288`, the trait default at `:300-302`). It is registered at `crates/cgg-lang/src/plugins.rs:126`.

## 01. Detection & parsing

- **Extension `.kv`**: this is the only detection route (`kivy.rs:76-78`). Fixture `a/MainWidget.kv` was reported with `detected_via: "extension:.kv"`.
- **Shebangs**: none (`kivy.rs:79-81`, returns `&[]`).
- **Content detection**: none. `cgg-lang/src/detect.rs` has no kivy/kv rule (a grep found nothing).
- **Grammar**: the vendored `tree-sitter-kivy` (`vendor/kivy/grammar.js`). It has these top-level productions: `directive` (`#:…`), `section`, `comment`, `rule` (`<Sel[,Sel]>[:]`), `widget` (`Name:` + block), `canvas_block` (`canvas`/`canvas.before`/`canvas.after`), and `property` (inline, multiline, `property_block`, empty) (`grammar.js:37-200`). Expressions are a flat sequence of identifiers, punctuation and parenthesized/list/dict expressions, so the grammar has no structured call node (`kivy.rs:5-7`).
- **Long-line guard**: `max_line_bytes() = Some(32_768)` (`kivy.rs:85-87`). The driver checks the longest line before parsing and skips the file as `SkipReason::LongLine(cap, longest)` (`crates/cgg/src/lib.rs:430-442`). The reason is that the grammar parses in time quadratic in line length. Kivy is the only plugin with a cap (CHANGELOG "Per-language line-length guard"; README:1456-1458). Test: `crates/cgg/tests/kivy.rs::kv_file_with_very_long_line_is_skipped_not_hung`.
- **Depth guard**: the generic `exceeds_depth(MAX_TREE_DEPTH=4000)` applies → `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:447-452`). It is not Kivy-specific.
- **Parse-error recovery**: there is no re-parse. Instead the walker uses several layers:
  - It walks `ERROR` nodes' well-formed children and then byte-scans the leftovers into a `kv_suite` def (`kivy.rs:200-207`, `record_anonymous_suite` `:341-394`).
  - A text pass, `harvest_unparsed_rule_calls` (`:694-778`), runs after the AST walk. It finds top-level `<Rule>`/`<Rule@Base>` headers by byte scan (`top_level_kv_rules`, `:627-685`). It re-attributes `root.`/bare calls to that rule and mints `kv_rule` nodes for calls nothing encloses.
  - Tests: `later_rule_keeps_root_calls_when_an_earlier_widget_does_not_parse`, `error_node_if_suite_extracts_play` (inline tests in `kivy.rs`).
- **Python statements inside bindings**: `if`/`else`/`elif` suites parse as widgets named `if`/`else`. The walker detects them (`is_python_keyword`, `:903-927`) and harvests their calls instead of treating them as owners (`:174-178`). Tests: `statement_if_extracts_the_call_after_the_colon`, `indented_if_else_suite_extracts_both_calls`, `if_else_apply_captured_by_on_release`.
- **Minified-file handling**: not applicable. KV has no minification concept, and the `locals` minified skip applies only to plugins that record locals.
- **parse_status**: a file with grammar errors is still extracted, with `parse_status: "error"`. Fixtures `d/a.kv` (`<-Foo>:`) and `d/b.kv` (`[Tmpl@BoxLayout]:`) both reported `error` and both still yielded nodes.

## 02. Callables extracted

Every node is `DefVariant::InherentMethod`, `Vis::Public`, attribute `kv-binding`. The qualified name is `<Owner>.<name>:<line>`.

- **Event binding (`on_*` property with at least one call)**:
  - Example: `<MainWidget>:\n    Button:\n        on_release: root.save()`. This produces `MainWidget.on_release:8`, with simple name `on_release` and signature `on_release: …` (`kivy.rs:270-297`).
  - The span is extended to cover leftover sibling `ERROR`/expression nodes (`property_scan_span`, `:784-822`) and `else:` widgets (`:830-833`).
  - Fixture `a`: 11 such nodes, including `on_touch_down:21` and `on_text:24`.
- **Multi-line suites**: `on_press:\n    root.load(2)\n    root.toggle() if True else root.save()` becomes one node spanning lines 9-12 (fixture `a`, `MainWidget.on_press:9`, 3 edges). Test: `indented_on_release_suite_extracts_each_call`.
- **`kv_suite` node**: minted for calls inside an `ERROR` node or an `if`/`else` "widget" (qualified name `<Owner>.kv_suite:<line>`, `:363-374`).
- **`kv_rule` node**: covers a `<Rule>` whose calls have no enclosing binding, for example `text: root.get_setting('x')`. Name `<Rule>.kv_rule:<line>`, spanning the rule (`:715-729`, `:759-773`). Fixture `a` produced `MainWidget.kv_rule:4` for `text: str(root.load(1))` (lines 4-24). Tests: `property_expression_call_gets_enclosing_kv_rule_node` and `tests/kivy.rs::kv_property_expression_call_keeps_method_live`.
- **Non-event property with calls after an event binding**: no node of its own. Its refs extend the preceding `on_*` def's span, clamped to the enclosing widget (`:298-325`). Test: `last_event_def_does_not_leak_across_rules`.
- **How the owner is chosen** (`owner_class`, `:141-150`), in order:
  - the innermost `<Rule>` selector name (only the **first** selector, `first_named_child_of_kind(node,"selector")`, `:154`);
  - else the file stem, if it starts with an uppercase letter or `_` (`:98-103`, test `filename_class_when_no_rule`);
  - else the first top-level widget name (`root_widget`, `:181-186`; fixture `b/screens.kv` with top-level `Root:` gave `Root.on_release:3`);
  - else the literal `kv` (`:257-262`).
- **Dynamic classes `<Custom@Button>`**: the owner is `Custom` (fixture `a`: `Custom.on_release:26`). The `@Base` part is not recorded as a base.
- **Not extracted / partial:**
  - Properties without calls (`text: 'Ok'`, `size: dp(12)`) produce no node, by design (`:263-269`).
  - Paren-less handler references (`on_press: root.by_ref`) produce no node and no ref. In fixture `b`, `Root.by_ref` got no edge.
  - Multi-selector rule `<A,B>:`: only `A` owns the bindings. In fixture `b` only `A.shared` was linked and `B.shared` got no edge.
  - Template rules `[Tmpl@BoxLayout]:` parse as an error, and the owner becomes the base `BoxLayout`. In fixture `d/b.kv` the node was `BoxLayout.on_release:2`, which was then unresolved.
  - `canvas:` instruction properties: no node. Their calls have no enclosing callable (fixture `b`: `canvas_fn` → `no-enclosing-callable`). This is a listed blind spot (`deadcode/caps.rs:59-66`).
  - Python `def` inside KV: not applicable (the grammar does not support it).
  - KV embedded in Python `Builder.load_string('''…''')` is not seen (fixture `b`: `Inline.inline_cb` got no edge; CHANGELOG "Known limitations").
  - Overloads, constructors, closures, operators, abstract members: not applicable to KV.

## 03. Call-site forms

References carry `name`, `receiver_hint` (the dotted chain before the name) and `context` (a preferred Python owner, from `preferred_owner`, `:397-427`). Two extractors run and are de-duplicated by `(byte, name)`: an AST walk over the flat `expression` (`scan_expression_parts`, `:475-547`) and a byte scanner (`scan_text_calls`, `:554-620`, which skips `#` comments and quoted strings).

- **Bare call** `free_fn()`: receiver `""`, context = rule. Fixture `a`: `on_release:19 → main.free_fn`, medium.
- **`root.m()`**: context = rule class. Fixture `a`: `on_release:8 → MainWidget.save`, high. Test: `extracts_root_method_call_from_rule`.
- **`self.m()`**: context = the innermost widget class (`:402-404`). `self.parent.save()` has no usable segment, so the context is empty and the linker refused it (fixture `a`: `kv-python: 2 candidates, no owner match`).
- **`app.m()`**: context `App` (`:405-407`). Fixture `a`: `app.quit_app() → MyApp.quit_app`, high.
- **`app.root.m()`**: context `app.root`, a sentinel for the widget `App.build()` returns (`kivy_app_context`, `ffi.rs:490-549`). Fixture `a`: `app.root.toggle() → MainWidget.toggle`, high. Tests: `app_root_receiver_is_a_linker_sentinel`, `ffi.rs::app_root_links_to_the_widget_app_build_returns`, `tests/kivy.rs::kv_app_root_resolves_to_build_return_type`.
- **Field chain `root.helper.go()`**: the context is the last segment, capitalised (`Helper`). It is refined through Python `class_fields` when the field's type is not a `*Property` descriptor (`refine_owner_via_fields`, `ffi.rs:567-605`). Fixture `a`: `→ Helper.go`, high.
- **Chained calls**: `root.act(root.nested())` records both calls, because the scanner recurses into arguments (`:603-609`). Fixture `b` produced 2 edges.
- **Ternary / boolean**: `root.a() if c else root.b()` records both calls. Test: `extracts_ternary_with_two_calls`, `nested_ternary_in_play_args_still_extracts_play_and_apply`.
- **`;`-separated statements**: `root.go(); root.go()` records two sites (fixture `d/c.kv`: two edges).
- **Lambda bodies**: `on_press: lambda x: root.save()` records the call (fixture `a`: `on_press:20 → MainWidget.save`).
- **`*args`**: `root.go(*args)` is recorded (fixture `d/c.kv`).
- **Skipped callees**: metric helpers and Python builtins (`dp`, `sp`, `str`, `int`, `len`, `max`, `print`, `super`, …; 34 names in `SKIP_CALLEES`, `:32-66`) are never recorded. Test: `skips_metrics_helpers`.
- **Not recorded or not supported:**
  - Paren-less handler references (`on_press: root.by_ref`): no `value_ref`/`paren_less` support.
  - String-dispatched events (`root.dispatch('on_custom')`): recorded only as a call to `dispatch`, unresolved (fixture `b`: `no-candidate-cross-file`).
  - Explicit generics, call operators, arity (`RefRecord::arity` is never set), registrar arguments, `typed_receiver`, `path_receiver`: none of these are set (a grep of `kivy.rs` for `arity|value_ref|typed` finds nothing).
  - `ids.<id>.m()` chains: the context skips `ids`, so `root.ids.lbl.save()` becomes context `Lbl` and is usually unresolved (fixture `a`; blind spot "ids.foo lookups", `caps.rs:61`).

## 04. Imports & module resolution

- **`#:import name module`**, **`#:include file.kv`** and **`#:set`**: these parse as `directive` tokens (`grammar.js:98`) and the plugin ignores them. `kivy.rs` never touches `imports`/`packages`. Consequences:
  - In fixture `a`, `#:import utils kivy.utils` followed by `utils.get_color_from_hex(...)` gave `no-candidate-cross-file`, and the audit placed it in the external bucket.
  - `#:include` is not followed (blind spot, `caps.rs:62`).
- **`FileFacts::packages`, manifest reading**: none.
- **`heads.rs` external-module refusal / `modules.rs` `not-in-imported-module`**: not applicable. Neither file mentions kivy, and KV refs carry no import table.
- **Cross-file target**: the kv-python linker considers **only callables whose language is `python`** that share the simple name (`ffi.rs:317-321`). There is no KV→KV linking and no KV→other-language linking.
- KV refs also flow through the generic intra-file/cross-file resolvers, which find nothing (`no-candidate-in-file`). The audit reconciliation leaves those records in the per-file `external_calls`/`unresolved_calls` buckets, in addition to the linker's own records (fixture `a` audit).

## 05. Type inference & receiver typing

- **Heuristic receiver typing only** (`preferred_owner`, `kivy.rs:397-427`): `root` → rule class, `self` → widget class, `app` → `App`, `app.root` → build-return sentinel, and otherwise the capitalised last non-keyword segment.
- **Field-type refinement**: Python `class_fields` (`name = Type(...)` / annotations) refine `root.<field>.m()`. `*Property` descriptor types are skipped, and the rule class is preferred over any other class declaring the field (`ffi.rs:283-296`, `:567-605`; CHANGELOG "refine_owner_via_fields resolved to the Kivy descriptor type").
- **`App.build` return**: the linker reads constructor refs inside `build` of classes whose base's last segment is `App` (case-insensitive). If there are none, it falls back to edges from `build` into `__init__`/constructors (`ffi.rs:490-560`).
- **None of the following**: `local_types`, `scoped_types`, `declared_type` wrappers/nullability, casts/constructions (`typed_receiver`), closure-parameter typing, `implicit_receiver`, `weak_receiver_type`, return-type maps. The plugin never populates them.

## 06. Inheritance, dispatch & overloading

- **Base classes**: not recorded. `<Custom@Button+Mixin>` bases are parsed by the grammar (`selector` `base` field, `grammar.js:110-121`), but `kivy.rs` reads only the `name` field (`:155`).
- **Owner matching**: an exact owner match scores 4, a case-insensitive match scores 3, and for `app.` any class whose name ends in `App` scores 2 (`kivy_owner_score`, `ffi.rs:450-484`). There is **no base-chain walk**, so a KV rule for a subclass that calls a method inherited from a Python base class does not match by owner. If that name has exactly one Python candidate, the name-only fallback binds it; otherwise it is refused. This is inferred from the code, not fixture-verified.
- **Not applicable**: `super`/`base`, `member_of_calling_class`, `overrides_below`, extension methods, mixins, arity narrowing. Kivy is not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS` (no `kivy` string in `dispatch.rs` or `type_hints.rs`).

## 07. Scoping & shadowing

- **`FileFacts::locals`**: not populated (not in `locals.rs`; `kivy.rs` never sets it).
- **`shadowed-by-local`, lexical narrowing, positional shadowing, conditional definitions**: none.
- **Rule/widget nesting**: tracked only to pick owners. `rule_stack`/`widget_stack` save and restore `last_event_def` so spans do not leak across rules and widgets (`kivy.rs:157-163`, `:187-193`; test `last_event_def_does_not_leak_across_rules`).

## 08. Macros, preprocessor & codegen

- **Not applicable**: KV has no macros. `#:set name value` defines KV-level constants; it is ignored as a directive and not expanded.
- **`name_mentions`**: none.
- **`macro:<name>` tagging**: none.
- **Generated code**: KV in Python strings is not extracted (see 02).

## 09. FFI & cross-language links

- **`kv-python` (the plugin's entire reason to exist)**: `link_kivy_python` (`ffi.rs:268-447`), called from `link_ffi` (`ffi.rs:178`). The process for each KV ref:
  1. Collect the Python candidates by simple name (`:317-321`).
  2. Find the enclosing KV callable (`:333`), the smallest span (`enclosing_callable`, `:608-625`).
  3. Score each owner (see 06). Ties at the best score are capped by `--fanout-cap` (`:411-423`).
  4. If nothing scores, use the name-only fallback (`:367-399`). It binds only when there is exactly one Python candidate, that candidate is not a test (`test_role.is_none()`), and the receiver head is `""`, `root`, `self`, `app` or `App`.
  - Edges carry `Via::Ffi("kv-python")`. Mermaid label: `ffi`.
- **Audit reasons for refusals** (CHANGELOG "kv→python unresolved references are audited"; test `tests/kivy.rs::kv_unresolved_reference_appears_in_audit`):
  - `no-candidate-cross-file`: no callable of that name, or none in Python;
  - `no-enclosing-callable`;
  - `fanout-cap-exceeded`;
  - `other: "kv-python: N candidates, no owner match"`.
- **Other FFI families** (PyO3, wasm-bindgen, napi, JNI, P/Invoke, extern C, asm): not applicable.
- **Descriptor links** (protobuf etc.): not applicable.
- **Markup code-behind** (`plugins/markup.rs`, `FileFacts::markup_bindings`): C# only (XAML/Razor/Godot). Kivy does not use it; its markup link is the FFI pass above.

## 10. Frameworks & entry points

Rules whose `language` is `"kivy"`: **none**. `RuleSpec` has one `language` per rule (`rules.rs:28-31`). The run in fixture `a` printed `no rules  1 file(s) in languages with no framework rules (kivy)`.

The rule that serves Kivy applications targets the **Python** side:

- **`kivy`**: `language: "python"`, `kind: TrustKind::Lifecycle`, `detect: ["kivy"]` (the file imports kivy), `node: false` (root-marks only, mints no node), `handler_on_enclosing_owner: true` (`crates/cgg-core/src/frameworks/rules.rs:290-414`). Its triggers:
  - `registrars`: `bind`, `fbind`, `schedule_once`, `schedule_interval`. Handlers passed by value are rooted, preferring the same class (tests `kivy_bind_and_app_build_are_framework_roots`, `kivy_bind_prefers_same_class_when_handler_names_collide`, `frameworks/mod.rs::value_ref_registrar_prefers_handler_on_enclosing_class`).
  - `base_types`: 33 widget/app bases (`App`, `Widget`, `Popup`, `Screen`, `BoxLayout`, …, `FileChooser`).
  - `methods`: 50 lifecycle and observer names (`build`, `on_start`, `on_stop`, `on_pause`, `on_resume`, `on_touch_*`, `on_kv_post`, `on_open`/`on_dismiss`, `on_value`, `on_text`, `on_press`, `on_release`, …) on subclasses of those bases (tests `kivy_slider_and_settingitem_lifecycle_methods_are_live`, `frameworks.rs::shape_d_kivy_app_build_marks_a_root_without_minting_a_node`).
  - `observer_types`: 14 `*Property` types. A class field `foo = StringProperty()` roots `on_foo` (tests `kivy_property_observer_is_live`, `frameworks/mod.rs::observer_callback_matches_a_declared_property_field`, `observer_callback_does_not_fire_without_a_matching_field`).
- **KV-side entry**: every kivy callable is a dead-code root `RootKind::LifecycleCallback`, rule `kivy:kv-binding` (`crates/cgg-resolve/src/deadcode/roots.rs:725-737`). This is a dead-code root, not a `<framework-entry>` node.
- **Not applicable**: `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`. No kivy entries (grep of `rules.rs` for `kivy` hits only lines 48, 64, 291, 294, 358).
- **Registrar verbs** (`ExtractCtx`): ignored, because `extract` takes `_ctx` (`kivy.rs:91`).

## 11. Test harness entries

- **None.** No `kind: test` rule names kivy, and the plugin sets no `test_role`.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: does not include kivy.
- **Test-file gating**: not applicable.
- On the Python side, the linker's fallback refuses to bind a Python candidate that has a `test_role` (`ffi.rs:380`).

## 12. Dead-code analysis

- **Roots**: every kivy callable (`on_*`, `kv_suite`, `kv_rule`) is a lifecycle root, `kivy:kv-binding` (`roots.rs:725-737`). The fixture `a` report listed 12/12 kivy callables as roots, 100% reachable.
- **Capability row** (fixture `a` `--dead-code-format json`): `class: degraded`, `visibility: none`, `attributes: full`, `value_references: none`, `dispatch: none`, `exports: none`, `test_tagging: none`, `max_confidence: medium`.
- **Blind spots** named in the report (`deadcode/caps.rs:59-66`): `ids.foo` lookups, canvas instruction properties, `#:include` not followed, KV in `Builder.load_string`.
- **Effect on Python findings**: KV edges keep the Python methods they reach live. Tests: `kv_binding_keeps_python_method_live`, `kv_indented_on_release_keeps_python_methods_live`, `kv_statement_if_keeps_python_methods_live`, `kv_property_expression_call_keeps_method_live`.
- **Gap: unresolved KV sites are not counted as doubt for Python methods.** `UnresolvedIndex::sites_named` is keyed by `(language, name)` (`deadcode/evidence.rs:102-106`, `:122-126`). In fixture `a`, the unresolved KV sites `self.parent.save()` and `root.ids.lbl.save()` did not stop `main.OtherWidget.save` being reported with evidence `no-unresolved-site-with-name` (it was capped at medium only by `implements-external-contract: BoxLayout`).
- **Not applicable**: visibility, exports, `macro-generated`, `reflection` caps, unreachable-code signals. All signals are false.

## 13. Builtins & stdlib tables

- **No `crates/cgg-core/src/stdlib/kivy.txt`** (the `ls` of the stdlib directory has no match), and no kivy entry in `stdlib.rs`.
- **Plugin-local filter**: `SKIP_CALLEES` (34 names: KV metric helpers `dp`, `sp`, `pt`, `mm`, `cm`, `inch` and colour helpers `rgb`, `rgba`, plus Python builtins). These calls are dropped at extraction (`kivy.rs:32-66`), so they never appear as unresolved or stdlib.
- **`CAST_INTRINSICS`/`CONSTRUCTING_METHODS`, implicit packages, `rust_types`**: none.
- **`--include-stdlib`**: no kivy stdlib sites exist.
- **`--include-external`**: mints `<external>::…` leaves for KV refs in the external bucket, including non-external ones. Fixture `a`: `<external>::root::missing_method`, `<external>::self.parent::save`, `<external>::root.ids.lbl::save`, `<external>::utils::get_color_from_hex`. A method missing from the workspace is presented as external (a quirk).

## 14. Confidence & certainty

KV edges do not use `CertainCtx`/`OwnerFacts`. Confidence comes from `link_kivy_python`:

- **`high`**: exactly one best candidate with score ≥3, meaning an exact owner match or a case-insensitive match, `App` subclass, or `app.root` build-return (`ffi.rs:424-428`). Fixture `c`: `root.controller.stop() → Controller.stop`, high, through capitalising `controller`. Owner match is by name only, so a same-named class anywhere in the tree scores the same.
- **`medium`**: several tied best candidates (≤ fanout cap), score 2 (`app.x()` → an owner ending in `App` that is not a detected App subclass; fixture `c`: `app.helper() → MainApp.helper` medium), or the name-only fallback with a bare head (fixture `a`: `free_fn()` medium; fixture `c`: `only(1)` medium).
- **`low`**: the name-only fallback with a rooted chain (`root.zzz.bar()`), `ffi.rs:381-385`. Fixture `c`: `root.zzz.bar() → Thing.bar`, low, an unrelated class.
- **Questionable medium binding**: `root.only()` bound to the top-level Python function `m.only` at medium (fixture `c`), even though `root.` denotes a method of the rule class.
- **Mermaid `?` marker**: applies normally. Fixture `a`: the medium edge `on_release:19 → free_fn` rendered `-->|?ffi|`, and high edges rendered `-->|ffi|`.
- **`--fanout-cap 0`**: refuses every scored KV binding, because `best.len() > 0` (`ffi.rs:411`). Inferred from the code, not fixture-verified.

## 15. Dynamic use & reflection

- **None.** The `dynuse` screens cover Python, JS, Java, C#, Ruby and PHP only, and `kivy.rs` records no `dyn_uses`.
- **String-named dispatch**: `root.dispatch('on_x')`, `Factory.<Name>`, `ids['x']` and `getattr` in KV expressions are not interpreted. In fixture `b`, `dispatch` was unresolved and `Root.dispatched` had no edge.

## 16. Known gaps & limitations

- KV in Python strings (`Builder.load_string`) is not seen (CHANGELOG known limitations; `caps.rs:64`; fixture `b`).
- `#:include` is not followed, and `#:import` aliases are not resolved (`caps.rs:62`; fixture `a`).
- `ids.<id>.method()` and `canvas:` instruction calls are blind spots (`caps.rs:60-61`). Canvas calls give `no-enclosing-callable` when the rule is a top-level widget rather than `<Rule>` (fixture `b`).
- Multi-selector rules `<A,B>:` bind only to the first selector (`kivy.rs:154`; fixture `b`).
- Template rules `[Name@Base]:` are a parse error, and their owner falls back to the base widget (fixture `d/b.kv`).
- Rule bases (`@Base+Mixin`) are not recorded, and the linker does no base-chain walk for inherited Python methods.
- Paren-less handler references (`on_press: root.cb`) and `dispatch('on_x')` produce no edges.
- The name-only fallback ignores the receiver: `root.x()` can bind a top-level function (medium), and `root.a.b()` can bind any single same-named method (low) (fixture `c`).
- Owner matching is case-insensitive and by simple class name, so same-named Python classes in different modules tie (refused only above `--fanout-cap`).
- Unresolved KV sites do not count as dead-code doubt for same-named Python methods, because doubt is keyed by language (`evidence.rs:102-126`; fixture `a`).
- `--include-external` labels workspace-missing KV methods as `<external>` (fixture `a`).
- There is no parse-time budget, only the 32 KB line cap. CHANGELOG: "the real fix is a parse-time budget, which is a pre-existing gap".
- `PluginSignals` are all false, so the dead-code class is `degraded` and findings are capped at medium.
- No KV-file test harness or `test_role`.
- Not in the README benchmark sweep as a separate "kivy" row besides `Carvera_Controller` (`scripts/update-readme-stats.sh:102`). Eight Kivy apps are listed in `scripts/benchmark.sh` `APPS` (lines 201, 331-346, 377).
- No `#[ignore]`d tests: neither `kivy.rs` nor `tests/kivy.rs` has `#[ignore]` (grep).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.kv` extension only; vendored grammar; 32 KB line cap; text-scan recovery after parse errors |
| 02. Callables extracted | partial | `on_*` bindings, `kv_suite`, `kv_rule` nodes; first selector only; no templates/canvas |
| 03. Call-site forms | partial | bare/`root`/`self`/`app`/`app.root`/chains/nested/ternary/lambda; no paren-less refs, ids, dispatch |
| 04. Imports & module resolution | none | `#:import`/`#:include` ignored; targets are any Python callable by name |
| 05. Type inference & receiver typing | minimal | receiver keyword heuristics + Python `class_fields` refinement + `App.build` return |
| 06. Inheritance, dispatch & overloading | none | no bases recorded, no base-chain walk, no arity |
| 07. Scoping & shadowing | n/a | no locals; rule/widget stacks only bound spans |
| 08. Macros, preprocessor & codegen | n/a | `#:set` ignored; `load_string` KV not extracted |
| 09. FFI & cross-language links | full | dedicated kv→python linker with owner scoring, cap and audited refusals |
| 10. Frameworks & entry points | partial | no kivy-language rule; Python-side `kivy` lifecycle rule; all KV callables are roots |
| 11. Test harness entries | none | no test rules or test roles |
| 12. Dead-code analysis | partial | KV roots keep Python live; class degraded; KV unresolved sites are not doubt for Python |
| 13. Builtins & stdlib tables | minimal | plugin-local `SKIP_CALLEES` (34); no stdlib file |
| 14. Confidence & certainty | partial | high/medium/low from owner score; name-only fallback can bind unrelated targets |
| 15. Dynamic use & reflection | none | no dynuse; `dispatch('on_x')` unresolved |
| 16. Known gaps & limitations | n/a | 15 gaps listed, fixture-confirmed where noted |
