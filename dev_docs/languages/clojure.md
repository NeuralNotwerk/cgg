# clojure

The Clojure plugin (`ClojurePlugin`, `crates/cgg-lang/src/plugins/clojure.rs`, 800 lines including 21 inline tests) parses Clojure, ClojureScript and `.cljc`/`.edn` files with the crates.io grammar `tree-sitter-clojure-orchard`. The workspace pins `"0.2"` (Cargo.toml:90), resolved to 0.2.5 (Cargo.lock:1448-1449), and the plugin uses `tree_sitter_clojure_orchard::LANGUAGE` (clojure.rs:21-23). Nothing is vendored.

A single recursive `ClojureWalker` (clojure.rs:45-476) looks only at `list_lit` forms (clojure.rs:65-79):

- An `(ns …)` form sets the file's namespace and records its `:require`/`:use`/`:import` entries as `ImportRecord`s.
- A `(def* name …)` form whose head is one of 13 symbols becomes a `FreeFunction` named `<ns>/<name>`.
- Every other list whose head is a `sym_lit` that does not start with `:` becomes a bare `RefRecord`. The whole symbol text, `alias/f` included, goes into `name`, and `receiver_hint` stays empty.
- When the head is a framework registrar verb (Compojure), the plugin also captures the route string and handler arguments, and it synthesizes `handler_at_<line>` closures for handlers written in place (clojure.rs:291-475).

The plugin overrides neither `signals()` nor `max_line_bytes` nor `ts_language_for`, so every `PluginSignals` bit is false: no visibility, attributes, exports, test_defs, value_refs, dyn_uses, unreachable or impls (lib.rs:269-306). It reads `ExtractCtx` only for the registrar gate (`ctx.is_registrar_verb`, clojure.rs:292). It never reads `reference_sites`. There is no `resolver_kind` method on the trait, so resolution goes through the generic pipeline. Two Clojure-specific branches exist in `cross_file.rs`:

- the shared `"require"` import arm (cross_file.rs:1846-1892);
- the `alias/f` exact-namespace lookup (cross_file.rs:5524-5539).

Fixtures cited below live under `<scratch>/parity/clojure/` (p1 … p5). Each was run with `target/release/cgg` 0.11.0 using `-t json -o pN.json`, optionally with `--include-external --include-stdlib`. The results were dumped with `show.py` in the same directory. `dc.json` is the `--dead-code --dead-code-format json` report for p1.

## 01. Detection & parsing

- **Extensions `.clj`, `.cljs`, `.cljc`, `.edn`** (clojure.rs:15-17; test `plugin_loads`, clojure.rs:507-512). p1 reports `detected_via: extension:.clj` / `.cljs` / `.cljc`. `deps.edn` → `extension:.edn` with 0 callables, which shows that data files are parsed as code.
- **No shebangs** (clojure.rs:18-20). Babashka scripts (`#!/usr/bin/env bb`, extensionless) are skipped as `unknown-extension` (p1 `script`, audit `file_skipped`). `.bb` is not a registered extension.
- **No content detection.** Clojure has no entry in `cgg-lang/src/detect.rs`: `grep clj/edn` finds nothing there.
- **One grammar for every dialect.** `.cljs` and `.cljc` use the same grammar, and there is no `ts_language_for` override. Reader conditionals `#?(:clj (a) :cljs (b))` are walked as plain children, so **both** branches produce edges (p1: `app.shared/plat -> inner-clj` and `-> inner-cljs`, both `high`).
- **No re-parse or fallback on parse errors.** The tree-sitter ERROR tree is walked as-is (test `unbalanced_source_does_not_panic`, clojure.rs:671-676).
- **Skip conditions.** Only the generic ones apply:
  - `MAX_TREE_DEPTH` = 4,000 → `SkipReason::TooDeep` (crates/cgg/src/lib.rs:155, 451-454).
  - No `max_line_bytes` limit (default `None`, lib.rs:313).
  - Clojure has no locals table, so the minified-file locals skip (lines over 2,000 bytes) does not apply.
- **Test-file classification** is generic and path-based: p1 `test/app/core_test.clj` gets `test_role: directory`, and its `defn test-helper` gets `test_role: support`.

## 02. Callables extracted

- **13 `def*` heads become callables** (`is_defn_form`, clojure.rs:101-123): `defn`, `defn-`, `defmacro`, `defmacro-`, `def`, `defonce`, `defprotocol`, `definterface`, `deftype`, `defrecord`, `defstruct`, `defmulti`, `defmethod`. The test `every_def_form_is_recognised` (clojure.rs:525-550) checks 12 of the 13; `defmacro-` is not exercised.
  - All of them get `DefVariant::FreeFunction` (clojure.rs:225), so JSON `kind` is `function`. A `def` of a non-function (`(def ^:dynamic *dyn* 1)`, `(def handler-val inner)`) is still a "callable" (p1).
  - The span covers the whole form (test `definition_spans_cover_the_whole_form`, clojure.rs:645-661).
  - `signature_hint` = `extract_signature(form text)` (plugins.rs:74). For Clojure that is usually the entire form, because no `{` appears at depth 0. Example from p1: `"(defn run [] (u/helper 1) …)"`.
- **Qualified name** = `<ns>/<name>` when an `ns` form came earlier in the file, otherwise the bare name (`qn`, clojure.rs:57-63). Tests: `the_namespace_qualifies_every_definition`, `without_a_namespace_the_bare_name_is_used`.
  - `ns` metadata (`(ns ^{:added "1.0"} a.b)`) is stripped through the grammar's `name` field (clojure.rs:128-130; test `namespace_metadata_is_not_part_of_the_name`, clojure.rs:790-799; CHANGELOG.md:233-235).
  - Only the most recent `ns` form applies, because `self.namespace` is overwritten. Two `ns` forms in one file therefore re-qualify everything after the second one (unverified by fixture; follows from clojure.rs:131).
- **Owners.** No definition has an owner. Every def is a free function under the namespace, and the namespace acts as the "enclosing owner" (frameworks/mod.rs:1514-1520 comment).
- **Not supported / partial:**
  - **Metadata on the defined name is kept as part of the name (bug).** `(defn ^:private hidden …)` → simple name `^:private hidden`, qn `app.util/^:private hidden`. The same happens with `^String typed`, `^:dynamic *dyn*` and `^:export init` (p1, p4). The plugin uses `named_child(1)` text verbatim (clojure.rs:216-217). Calls to `(hidden)` / `(typed)` then miss and fall to `<external>::hidden` (p4).
  - **Multi-arity `defn`** (`(defn multi ([] …) ([x] …))`) is ONE callable. Arities are not split and no arity is recorded (p1 `app.core/multi`, one node).
  - **`defmulti` + `defmethod`.** Each `defmethod` is a separate callable with the **same qn** as the `defmulti` (`app.core/area` ×3, p1). The dispatch value is not part of the name.
  - **Protocol and interface members are not callables.** `(defprotocol Speaker (speak [this]) …)` yields only `Speaker`, and `definterface IFoo (bar [])` only `IFoo`. Worse, the member signatures `(speak [this])` are recorded as **call sites** from `Speaker` (p1: `Speaker -> <external>::speak`, `-> shout`).
  - **`deftype` / `defrecord` inline method bodies are not callables.** The method bodies belong to the type's node: p1 shows `app.core/Dog -> app.util/helper` from `(shout [this msg] (u/helper msg))`, and `(speak [this] …)` is recorded as a call to `speak`.
  - **Not definitions at all:** `extend-protocol`, `extend-type`, `extend`, `reify`, `proxy`, `gen-class` methods, `letfn` locals, `fn` / `#(…)` lambdas, `deftest`, `defspec`, `defroutes`, `s/fdef`/`s/def` (spec), `declare`, and any user `def*` macro (`defn-traced`, `defsc`, …). Only the 13 hard-coded heads count.
    - Calls inside `extend-protocol` bodies have **no enclosing callable** (p1: `inner` at L18, `no-enclosing-callable`).
    - `deftest` is named explicitly as missing in the `clojure-test` rule gap (rules.rs:3363).
  - **Quoted templates.** A `defn` whose name starts with `~` (an unquote inside a syntax-quote) is skipped (clojure.rs:218-222; test `unquoted_defn_names_are_not_definitions`, clojure.rs:552-563; CHANGELOG.md:1052-1054). Other templates still count: a quoted `` `(defn literal-name …) `` inside a macro body would still be a definition (unverified; no syntax-quote check beyond the `~` prefix).
  - **Synthetic route handlers.** `handler_at_<line>` is a `NamedClosure` with attribute `synthetic` (clojure.rs:423-475), covered in §10.

## 03. Call-site forms

- **Bare call** `(inner x)` → `RefRecord{name:"inner"}` (clojure.rs:246-273; test `calls_inside_a_body_are_references`). It binds intra-file at `high` (p1 `run -> inner`).
- **Namespace-qualified / alias call** `(u/helper 1)` and `(app.util/helper 2)`. The whole symbol is the name with an empty receiver (clojure.rs:251-269), and the cross-file Clojure branch splits it on `/` (§04). p1: both → `app.util/helper` `high` `cross-file:imports`.
- **Keyword-as-function** `(:k m)` is never recorded (clojure.rs:252; test `keywords_in_call_position_are_not_references`).
- **Calls inside special forms** are recorded because the walker recurses into every `list_lit`: `->`, `->>`, `when-let`, `let`, `fn` and `letfn` bodies (p1, p4).
  - The special-form heads themselves (`let`, `fn`, `->>`, `when-let`) are recorded as calls and classified as `<stdlib>` (p1 with `--include-stdlib`).
- **Not supported / partial:**
  - **Threading-macro bare symbols.** In `(-> x dup)` the symbol `dup` is not a list, so no call is recorded. Only `(dup)` in list form is (p4 L9).
  - **Anonymous fn literal** `#(inner %)` → no call recorded. The `anon_fn_lit` head is not a `list_lit` (p1 L43: no edge). `((fn [x] (inner x)) 1)` does record `inner` (p1 L44).
  - **Higher-order / value references.** `(map inner xs)`, `(map u/helper xs)`, `(apply dup [])`, `(partial dup)`, `(comp dup dup)` and `(def handler-val inner)` record nothing for the function passed, even under `--reference-edges` (p1 L41-42, p4 L13). The plugin never sets `RefRecord::value_ref`. Its only value refs are the registrar captures in §10.
  - **Java interop.** All of these are recorded with the literal symbol as the name and end up `<external>` (p1 L36-39, L48-49; p4 L12): `(.toUpperCase s)` → name `.toUpperCase`; `(Math/abs x)` → `Math/abs`; `(String. "x")` / `(Date.)` → `String.` / `Date.`; `(->Dog …)` → `->Dog`. There is no receiver typing, and the record constructor `->Dog` is not linked to `defrecord Dog`.
  - **ClojureScript interop** `(js/console.log …)` and `(.log js/console …)` → external (p1 front.cljs).
  - **Discard reader `#_(gone)` still records the call:** p4 `caller -> gone` `high`, a false edge.
  - **`(comment (inner 1))` records the call:** p1 L53 `run -> inner`, `high`, a false edge. `comment` itself is external because it is not in the stdlib list.
  - **Non-call lists are recorded as calls:** protocol member signatures (`(speak [this])`), `letfn` binding forms (`(loc [] …)` → external `loc`, p1 L45), and `deftype`/`defrecord` method heads (`(bar [this] …)` → external `bar`, p4).
  - **No arity is recorded** (`arity: None` in every record), so there is no overload narrowing (§06).
  - **No explicit generics or operator overloading.** Not applicable to Clojure.
  - **No paren-less calls.** Not applicable.

## 04. Imports & module resolution

- **`(ns name (:require …) (:use …) (:import …))`** is read only inside the `ns` form (`extract_namespace`, clojure.rs:125-163). Each entry becomes an `ImportRecord` with kind `require` / `use` / `import` (clojure.rs:165-212; test `use_and_import_keep_their_own_kinds`).
  - `sym_lit` entry `a.b` → path `a.b`, no alias.
  - `vec_lit` entry `[a.b :as x]` → path is the first symbol, alias is the symbol after `:as` (tests `require_with_an_alias_records_path_and_alias`, `a_bare_require_vector_has_no_alias`).
- **How a `require` resolves** (cross_file.rs:1846-1892):
  1. Every definition of a file found by trying the paths `p`, `p.clj` and `p` with `.`→`/` + `.clj` (plus the Ruby/Lua variants) becomes reachable through `collect_include_defs` (budget 4).
  2. The alias, or else the namespace's last dotted segment, is put in `module_aliases`.
  3. The path is pushed to `unqualified_prefixes`.
- **`alias/f` and `full.ns/f`** (cross_file.rs:5524-5539) map the alias through `module_aliases`, then look up the exact qn `ns/f` in `by_qn`.
  - When found, it binds with `module_certain = true`, which gives `high` (test `a_clojure_alias_call_binds_in_the_required_namespace`, cross_file.rs:14427-14446; CHANGELOG.md:233-235 reports "+13,436 high alias calls", CHANGELOG.md:257-258).
  - When not found it returns `None` and the site never binds by name elsewhere. p2: `(u/missing)` and `(nope/z)` → `<external>`, and with no `--include-external` they are silently dropped (no unresolved record).
- **`:refer`, bare require and `:use`.** The `:refer [w]` list is not read; the required file's definitions become reachable wholesale.
  - p3: `[a.b]` without `:refer` → bare `(x)` binds `a.b/x` `high`. That is over-permissive: Clojure would reject it.
  - p3 `a.s`: `[a.d :refer [w]]` plus `[a.c :as-alias ca]` → bare `(y)` fans out `medium` to **both** `a.c/y` and `a.d/y`, though neither is referred.
  - `(:use a.b)` → `(x)` → `a.b/x` `medium` (p3).
- **Bare call with no import** finding same-named defs elsewhere → `candidates-in-other-files`, not bound (p3 `a.r/f y`, 3 candidates). By-name fan-out happens only through the import closure.
- **Not supported / partial:**
  - **Prefix lists** `(:require (a [c :as cc]))` and `[a [c :as cc]]` are not expanded: `cc/z` → external, and bare `(z)` → `candidates-in-other-files` (p3 `a.q`).
  - **`:as-alias`** is not recognised as an alias (`seen_as` only for `:as`, clojure.rs:190). `ca/z` → external (p3).
  - **Top-level `(require '[a.c :as k])`** / `(require (quote …))` outside `ns` → not an import. `k/z` and `dd/w` → external (p3 `a.r`). The same applies to `use`, `import`, `refer`, `load` and `alias` calls.
  - **Hyphen/underscore munging.** The require path tries do not map `my-lib.core` → `my_lib/core.clj`, nor `.cljc`/`.cljs` files. In p5, `[my-lib.core :refer [hy]]` → `(hy)` fans out `medium` to `my-lib.core/hy` **and** the unrelated `app.other/hy`, a wrong edge. The alias path is unaffected because it matches by qn.
  - **`:refer :all` / `:rename` / `:exclude` / `:only`** are not read.
  - **`(:import [java.util Date])`** is recorded (kind `import`, path `java.util`). Nothing consumes it for resolution.
  - **`FileFacts::packages` is not populated.** Clojure has no `packages`, and the namespace lives only in qns.
  - **No manifest reading.** `deps.edn`, `project.clj` and `shadow-cljs.edn` are parsed as Clojure code, not read as manifests (no Clojure branch in `plugins/manifest.rs`).
  - **No `heads.rs` external-module refusal.** Clojure is not in WorkspaceVocab, which covers Python/JS/TS/Go/Java/Kotlin/C#/C++ (CLAUDE.md).
  - **No `modules.rs` `not-in-imported-module`.** `modules.rs:190` lists js/ts/python/go/lua only. The closest equivalent is the alias branch's silent `None`.
- **Stdlib alias tables are keyed by conventional alias, not by namespace.** `stdlib/clojure.txt` lists `str/join`, `io/file` and `walk/…`. p2 results:
  - `(str/trim …)` → stdlib, even with no `clojure.string` require;
  - `(s/join …)` under `[clojure.string :as s]` → external;
  - `(clojure.string/join …)` → external.
- **Bug: a resolved alias call also mints an `<external>` exit node.** With `--include-external`, `(u/h)` produces both `a.m/f -> a.u/h high` and `a.m/f -> <external>::u/h low` (p2; also p1 L31-32, L14). The audit reconciliation does not prune the external bucket entry for sites the Clojure branch bound.

## 05. Type inference & receiver typing

- Not applicable / none. There is no Clojure code in `type_hints.rs`, `receiver.rs` or `closure_types.rs` (`grep clojure` finds nothing).
  - Type hints (`^String x`, `^T t`) are not read. Worse, a hint on the def name corrupts the name (§02).
  - `(.bar t)` with `^T t` → `<external>::.bar` (p4).
  - Locals, parameters, `let` bindings, record fields and return types are not recorded. There are no casts, constructions, wrappers, implicit receivers or weak receiver types.

## 06. Inheritance, dispatch & overloading

- **Base / implementation records: none.**
  - `defrecord Dog [name] Speaker …` records no `base_types`, `trait_impl_target`, `ClassDecl` or `classes` (p1).
  - `extend-protocol` / `extend-type` record nothing.
  - The dead-code capability table shows `disp no` (p1 dead-code report).
- **Protocol dispatch: not modelled.** `(speak (->Dog "rex"))` → `<external>::speak` (p1 L48-49), because protocol methods are not callables (§02).
- **Multimethods.** A call to `(area x)` binds to the `defmulti` **and every `defmethod`** at `medium`, through `cross-file:imports` even though it is in the same file (p1 L50: three `medium` edges, mermaid `?`). The dispatch value is not used. This is the only dispatch-like fan-out, and it happens by accident of shared qns.
- **Not a member of:** `INHERITANCE_LANGS` (dispatch.rs:110-123), `OVERLOAD_ARITY_LANGS` = java/scala/cpp (type_hints.rs:1583), clause languages = erlang/elixir/haskell (`is_clause_language`, intra_file.rs:858-860), or `member_of_calling_class` / `overrides_below`. `--dynamic-dispatch` adds nothing (p4 run with `--dynamic-dispatch`: no dyn edges).
- **No `super`/base calls, extension methods, mixins or arity narrowing.** Multi-arity `defn` is one node, and the self-call `(multi 1)` binds `multi -> multi` `high` (p1 L27).
- **Redefinition** (`(defn dup …)` twice) → `ambiguous-in-file`, unresolved. Clojure's last-def-wins rule is not modelled (p4 `use-dup`, `->thread`, `T`, `hof`).

## 07. Scoping & shadowing

- **`FileFacts::locals`: none.** `locals.rs` `spec()` has no Clojure entry (locals.rs:380-394).
  - `(let [inner (fn [] 0)] (inner))` still binds to the top-level `app.core/inner` at `high`, a wrong edge (p1 L47).
  - `(let [f inner] (f 2))` → `f` external (p1 L46).
  - `letfn` locals → external (p1 L45).
- **Lexical narrowing: no.** Clojure is not in `lexically_scoped` (intra_file.rs:116-134), and nested defs are not extracted anyway.
- **Positional shadowing: no.** That list is ocaml/fsharp/lua only (intra_file.rs:405). A later `(defn dup …)` does not shadow an earlier one (§06).
- **`bare_call_is_never_a_method`: not listed** (intra_file.rs:49-54). This has no effect, because Clojure defs carry no owners.
- **No conditional definitions.** `(when x (defn …))` would still be extracted, and `DefRecord::conditional` is never set (unverified by fixture; the walker extracts any matching list at any depth, clojure.rs:65-79).
- **Forward references** through `declare` work, because resolution is file-wide: `(declare later)` + `calls-later -> later` `high` (p1). `declare` itself becomes a ref.

## 08. Macros, preprocessor & codegen

- **`defmacro` / `defmacro-` are ordinary callables.** Calls to them bind like functions (p1 `uses-macro -> my-macro` `high`), and calls inside the macro's syntax-quoted template are attributed to the macro (`my-macro -> inner` `high`).
- **Macro expansion: none.** Code a macro generates is invisible, and nothing is tagged `macro:<name>`. Clojure is not in the `rust_macro`/`c_macro` paths.
- **Quote / unquote.** `defn ~n` templates are skipped (clojure.rs:218-222; CHANGELOG.md:1052-1054, 1079, 1517 "Clojure 7/7 (re-attributions)").
- **Reader conditionals** `#?` / `#?@`: both branches are walked (§01). The discard reader `#_` is not honoured (§03).
- **`name_mentions`: never populated** by the Clojure plugin.
- **Preprocessor / `#define` / `#if` / templates:** not applicable.

## 09. FFI & cross-language links

- **None.** `ffi.rs` has no Clojure handling (`grep clojure` finds nothing in ffi.rs). Java interop calls, `gen-class` exports, ClojureScript `js/` calls and `^:export` all go `<external>` (p1). The `^:export` metadata is not even an attribute; it lands in the name.
- **Descriptor links** (protobuf rpc → host): no Clojure host binding.
- **Markup code-behind:** not applicable.

## 10. Frameworks & entry points

Seven rules carry `language: "clojure"` (rules.rs:3254-3367). No multi-language rule names Clojure.

- **`compojure`**, kind `Network`, `detect: ["compojure"]`, registrars `get post put patch delete head options any context defroutes` (rules.rs:3254-3279). This is the only enumerating rule. `app-metabase-compojure` in benchmark.sh:200 is the verified app.
  - **Registrar capture** (`record_registrar`, clojure.rs:291-410). It is gated on `ctx.is_registrar_verb(last_segment(head))`. The builtin verbs are per language (lib.rs:110-130, 202-235) and also match case-insensitively, so `GET` matches. `compojure.core/GET` reduces to `GET` (test `a_namespaced_operator_reduces_to_its_verb`).
    - A **leading** `str_lit` is the route. A non-leading string never is: `(get m "k")` stays a map lookup (test `a_map_lookup_does_not_mint_a_handler`).
    - With a route present, slot 1 is the request binding and is skipped (test `the_request_binding_slot_is_not_a_handler`).
    - A `sym_lit` argument → a `VALUE_REF_HINT` record with `context=verb` and `route` set (test `a_compojure_route_captures_its_named_handler_and_path`). A non-leading string → a `STRING_REF_HINT` record.
    - Vectors and maps are ignored.
  - **Inline handlers** (`synthesize_inline_handler`, clojure.rs:423-475). If there is a route and no named target, the `list_lit`/`anon_fn_lit` body args become a `handler_at_<line>` `NamedClosure` (`synthetic`), and a value ref names it (tests `an_inline_route_body_becomes_a_named_handler`, `a_named_handler_suppresses_the_synthetic_one`).
  - **Fixture p1 `web.clj`** mints four entries, `get("/users")`, `get("/")`, `context("/api")` and `post("/x")`, with `framework_entry` edges at `low` to `list-users`, `handler_at_10` and `handler_at_12`.
    - Partial: `context` and the nested `POST` on one line both synthesize `handler_at_12`. That gives two callables with one name and two `value-ref-ambiguous` unresolved records (p1).
    - Partial: `(defroutes app-routes …)` records `app-routes` as a value ref with no enclosing callable (`value-ref-no-enclosing`, p1).
  - `handler_on_enclosing_owner: false`. The comment at frameworks/mod.rs:1514-1520 explains why: in Clojure the "owner" is the namespace, so `(get m id)` would bind `id`.
- **`ring`**, `Network`, `detect: ["ring"]`, detect-only gap rule (rules.rs:3281-3296). The gap: `run-jetty` has no argument capture. In p1 the file is detected ("seen, no rules"), and `start -> <external>::run-jetty`.
- **`http-kit`**, `Network`, `detect: ["org.httpkit"]`, gap rule (rules.rs:3298-3313).
- **`pedestal`**, `Network`, `detect: ["io.pedestal"]`, gap rule: routes are data (rules.rs:3315-3330).
- **`reitit`**, `Network`, `detect: ["reitit"]`, gap rule: `:handler` values sit inside literals (rules.rs:3332-3347).
- **`clojure-test`**, `Test`, `detect: ["clojure.test"]`, gap rule (rules.rs:3349-3367), covered in §11.
- **Detection** matches the `detect` prefixes against import paths. The test `crates/cgg/tests/detect_prefixes.rs:47` writes `(ns m (:require [<prefix>]))` for each Clojure rule and asserts that it is detected.
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `observer_types`, `base_types`, `attributes`, `methods`: none for Clojure.** The plugin emits no attributes for them to match.
- **Missing frameworks with no rule at all** (from catalogue knowledge; none appear in rules.rs): Integrant/Component/Mount lifecycle, re-frame (`reg-event-fx` / `reg-sub`), core.async `go` blocks, Luminus, Yada, Liberator, Duct.

## 11. Test harness entries

- **`clojure-test` is a detect-only gap rule** (rules.rs:3349-3367). `deftest`, `testing` and `defspec` are not definitions, so test bodies have no callable (p1 `core_test.clj`: `a-test` absent, and `(c/inner 1)` inside it produces no edge). Detection works: p1 lists `clojure-test` under "seen, no rules".
- **Name convention.** `is_test_name` falls to the default arm (`test_` / `Test` prefix, deadcode/roots.rs:975-988). An underscore name like `test_thing` becomes a `builtin:test-name` root (p1 dc.json). Idiomatic kebab names (`test-helper`, `foo-test`) do not match.
- **No test harness node** (`<framework-entry>::test::…`). Clojure is not in `TEST_CLASS_MEMBERS_ARE_ROOTS`, and there is no `TEST_ATTRS` for it.
- **Test-file gating** is generic and path-based (`test/` directory → `test_role: directory`/`support`, p1).
- **Missing:** kaocha, midje (`fact`), test.check (`defspec`), expectations, and cljs.test `async`.

## 12. Dead-code analysis

- **Language class `degraded`.** The capability row reads `vis no, attrs no, refs no, disp no` (p1 dead-code report). `signals_complete()` therefore fails (deadcode/caps.rs:89-96), and every Clojure finding is capped at **medium**. p1: "0 shown (high) · 18 withheld (medium)". Every finding carries `language-lacks-visibility`, `-attributes`, `-value-references` and `-dispatch-model` (dc.json).
- **Visibility is not extracted.** `defn-` and `^:private` are not recognised, and `visibility` is `""` (clojure.rs:239). `private-one` and `hidden` get the same evidence as public `unused-pub` (dc.json). There is no `PrivateVisibility` corroboration.
- **Roots that fire for Clojure** (dc.json):
  - `builtin:main` for a fn named `main` (UNIVERSAL_ENTRY, roots.rs:19).
  - `builtin:test-name` for `test_*` / `Test*`.
  - `toplevel:invocation` for any name called outside every callable (roots.rs:560-612). In p1, `inner` became a root by accident through the `extend-protocol` body, and `list-users` through the route capture at top level.
  - `framework:entry-node` for Compojure entries.
- **`-main` is not a root.** Clojure's `:gen-class` entry `-main` is reported as a `medium` finding (dc.json `app.core/-main`). `LANG_ENTRY` (roots.rs:23-32) has no Clojure row, and `LIFECYCLE` has none either.
- **Not roots:** `^:export` (cljs) functions (`app.front/^:export init` is a finding), protocol implementations, `defmethod`s, `deftest`s (they are not callables), and anything referenced only as a value (`(def handler-val inner)`, `(map f xs)`).
- **Exports: none** (`exports: false`), so there is no `ExportedApi` root.
- **Confidence caps.** `implements-external-contract` is never applied: Clojure has no override or contract records. `macro-generated` never applies (no `macro:` tags). The `reflection` cap never applies (no dynuse, §15).
- **Unresolved-doubt handling** is generic: every p1 finding shows `no-unresolved-site-with-name`. Value refs from registrars feed `value-ref-*` reasons.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/clojure.txt`, 367 lines**, loaded at stdlib.rs:211 and registered at stdlib.rs:252. It holds clojure.core names, including special forms and macros (`fn`, `let`, `letfn`, `->`, `->>`, `defn`, `deftest`, `reify`, `extend-protocol`, `proxy`, `ns`, `require`, `resolve`, `ns-resolve`, `load-file`).
  - It also holds alias-prefixed entries under conventional aliases only: `str/…` (about 20), `io/…` (9) and `walk/…` (4) (clojure.txt:143-151, 287-309, 352-355).
  - Missing: `eval`, `requiring-resolve` and `comment` (p1: `requiring-resolve` and `comment` → external).
- **How a name is classified.** `classify_external` (cgg-core/src/external.rs:144-…) splits on the last `.`/`:` and has no `/` rule. So `str/join` matches only as a literal entry, while `s/join` and `clojure.string/join` are external (p2).
- **No CAST_INTRINSICS, CONSTRUCTING_METHODS, implicit-package or rust_types-like tables** for Clojure.
- **`--include-stdlib`** mints `<stdlib>::<name>` leaf nodes (p1: `->>`, `map`, `filter`, `fn`, `let`, `letfn`, `resolve`, `str/join`). `--include-external` mints `<external>::…` for everything else, including the duplicate exit node for resolved alias calls (§04).

## 14. Confidence & certainty

- **`high` comes from:**
  - Intra-file single-candidate bare calls (p1 `run -> inner`, `-main -> run`).
  - Alias and fully-qualified calls through the Clojure branch (`module_certain = true`, cross_file.rs:5535; p1/p2 `u/helper`, `app.util/helper`).
  - Bare calls to a uniquely named def in a required namespace (p1 `(query)` → `app.db/query` `high`; p3 `a.p (x)` `high`).
- **`medium` comes from:**
  - Multi-candidate fan-out through the import closure (p3 `a.s (y)`; p5 `(hy)` with a wrong `app.other/hy`).
  - Multimethod `defmulti` + `defmethod` sets (p1 `area`).
  - `:use` bare calls (p3 `a.t`).
- **`low`** is used for framework-entry edges and exit nodes.
- **False `high` edges observed** in the fixtures:
  - `#_(gone)` (p4);
  - `(comment (inner 1))` (p1 L53);
  - `let`-shadowed `inner` (p1 L47);
  - bare call into a required namespace without `:refer` (p3 `a.p`);
  - the both-branches `#?` edges (p1 `plat`).
- **Mermaid** prefixes `?` on arrows with no `high` site (p3: `N9 -->|?| N2`; p1: `run -->|?| area`).
- **No `CertainCtx` / `OwnerFacts` paths** apply, because there are no typed receivers.

## 15. Dynamic use & reflection

- **None.** The plugin never calls `plugins/dynuse.rs` and `dyn_uses` is false. In p1:
  - `(resolve 'app.util/helper)` → only a stdlib `resolve` edge. The quoted symbol does not mark `helper` live.
  - `((requiring-resolve 'app.util/unused-pub))` → external `requiring-resolve`, and `unused-pub` stays a finding (dc.json).
- **`ns-resolve`, `find-var`, `var-get`, `#'var`, `eval`, `load-file`, `load-string` and `(apply (symbol …))`**: no screen and no cap. `DYN_MODULE_LOADERS` is `["require", "importScripts", "import_module"]` (cgg-core/src/facts.rs:165), but it applies only through dynuse, which Clojure does not use.

## 16. Known gaps & limitations

- **Framework gap strings** (rules.rs): ring (3293), http-kit (3310), pedestal (3327), reitit (3344), clojure-test (3363: "the Clojure plugin does not treat `deftest` as a definition at all").
- **Fixtures confirmed these gaps and bugs:**
  1. Name metadata (`^:private`, `^String`, `^:dynamic`, `^:export`) is kept inside the callable name, which breaks every call to such a function (p1, p4).
  2. `defn-` / `^:private` visibility is not recorded, so the language is `degraded` and every dead-code finding is capped at medium.
  3. Protocol, interface, `deftype`/`defrecord`/`reify`/`extend-*` members are not callables. Their signatures become fake call sites, and protocol calls go external.
  4. No value references: `(map f xs)`, `partial`, `comp`, `apply` and `def` aliases, plus `#(…)` lambda bodies and threading-macro bare symbols, are lost.
  5. No locals table: `let`/`letfn`/`fn` parameter shadowing produces wrong `high` edges.
  6. `#_` discard and `(comment …)` bodies produce false edges.
  7. Not understood: prefix lists, `:as-alias`, `:refer` filtering, top-level `(require …)`, and hyphen→underscore file mapping. The result is external drops or by-name fan-out to unrequired namespaces.
  8. With `--include-external`, a resolved alias call also emits an `<external>` exit edge (p2).
  9. `deftest`/`defspec` are not callables, and `-main` and `^:export` are not roots.
  10. `defmethod`s share the `defmulti` qn, and a call fans out to all of them at medium.
  11. Redefinitions are `ambiguous-in-file`, with no last-def-wins.
  12. No reflection screening for `resolve` / `requiring-resolve` / `ns-resolve`.
  13. `.edn` data files are parsed as code. `.bb` files and the `bb` shebang are not detected.
  14. The Compojure `context` + nested verb on one line mint two `handler_at_<line>` callables with one name (`value-ref-ambiguous`).
- **README** language table row: `| Clojure | (ns :require ...) | — | defn/defmacro/deftype/defprotocol |` (README.md:513). The benchmark repo is `clojure-ring` (`ring-core/src`, benchmark.sh:87): 209 callables, 212 edges, 8% cross-file (README.md:1339).
- **CHANGELOG follow-ups.** The perf note says Metabase's Clojure `require` graph made path normalisation cost 1.1 s (cross_file.rs:3426-3433). No open Clojure follow-up items were found in CHANGELOG.md beyond the fixed alias and metadata entries (CHANGELOG.md:233-235).
- **Ignored tests:** none. `grep '#\[ignore' clojure.rs` finds nothing, and none of the 21 inline tests are ignored.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | .clj/.cljs/.cljc/.edn by extension; no shebang/`.bb`; `#?` both branches walked; `.edn` data parsed as code |
| 02. Callables extracted | partial | 13 `def*` heads as free functions `ns/name`; metadata corrupts names; no protocol/type/reify/extend members, lambdas, letfn, deftest |
| 03. Call-site forms | partial | list-head calls incl. `alias/f`; no value refs, `#()` bodies, threading symbols; `#_`/`comment` and signatures produce fake calls |
| 04. Imports & module resolution | partial | `ns` `:require/:use/:import` with `:as`; exact `alias/f` binding (high); no prefix lists, `:as-alias`, `:refer` filtering, top-level require, `-`→`_` mapping |
| 05. Type inference & receiver typing | none | type hints ignored (and corrupt def names); no locals/receivers |
| 06. Inheritance, dispatch & overloading | none | no protocol/record impl records; multimethods fan out by shared qn; not in any dispatch/arity list |
| 07. Scoping & shadowing | none | no locals table, no lexical/positional shadowing; let-shadowing yields wrong high edges |
| 08. Macros, preprocessor & codegen | minimal | defmacro is a callable; `~name` templates skipped; no expansion, no `macro:` tags, no name_mentions |
| 09. FFI & cross-language links | none | Java/JS interop all external; no ffi or descriptor links |
| 10. Frameworks & entry points | partial | compojure enumerates (registrar capture + inline handler synthesis); ring/http-kit/pedestal/reitit are detect-only gaps |
| 11. Test harness entries | none | clojure-test is a gap rule; deftest not a callable; only `test_`/`Test` name roots |
| 12. Dead-code analysis | minimal | class degraded, all findings medium; no visibility/exports; `-main`/`^:export` not roots |
| 13. Builtins & stdlib tables | partial | 367-name clojure.txt incl. special forms; alias entries keyed by conventional alias only (`str/`, `io/`, `walk/`) |
| 14. Confidence & certainty | partial | high for intra-file unique and exact alias calls; several false-high shapes (`#_`, comment, let-shadow, no-refer) |
| 15. Dynamic use & reflection | none | no dynuse; resolve/requiring-resolve/ns-resolve unscreened |
| 16. Known gaps & limitations | n/a | 14 confirmed gaps listed; 5 framework gap strings; no ignored tests |
