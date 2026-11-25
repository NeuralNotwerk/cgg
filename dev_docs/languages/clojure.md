# clojure

> Revised 2026-11-25 against cgg 0.13.3: name metadata, protocol/type methods, multimethods, `letfn`, `#_`, `#(…)`, value references, the locals table, `:refer` scoping, `import-vars`, schema `defn`s and external-contract roots now work, so the stale gaps, fixture results and scorecard were corrected. `file:line` citations not otherwise revised date from the 0.11.0 audit and may have drifted.

The Clojure plugin (`ClojurePlugin`, `crates/cgg-lang/src/plugins/clojure.rs`, 2,450 lines including inline tests at 0.13.3) parses Clojure, ClojureScript and `.cljc`/`.edn` files with the crates.io grammar `tree-sitter-clojure-orchard`. The workspace pins `"0.2"` (Cargo.toml:90), resolved to 0.2.5 (Cargo.lock:1448-1449), and the plugin uses `tree_sitter_clojure_orchard::LANGUAGE` (clojure.rs:21-23). Nothing is vendored.

A single recursive `ClojureWalker` (`walk`, clojure.rs:133-192) handles `list_lit` and `anon_fn_lit` forms, skips `dis_expr` (`#_`) entirely, and under `reference_sites` also reads `quoting_lit` and `var_quoting_lit` (`#'f`):

- An `(ns …)` form sets the file's namespace and records its `:require`/`:use`/`:import` entries (inside reader conditionals too) as `ImportRecord`s, plus `refer`/`refer-all` records for `:refer [f]`/`:refer :all`/`:use`.
- A method list inside `deftype`/`defrecord`/`reify`/`proxy`/`extend-type`/`extend-protocol`, or a `defprotocol`/`definterface` signature, becomes a method (`method_spec`/`record_method`, clojure.rs:521-637).
- A `letfn` spec becomes a nested callable (`record_letfn`, clojure.rs:650).
- A `(def* name …)` form whose head is one of 13 symbols becomes a `FreeFunction` named `<ns>/<name>` (`record_callable`, clojure.rs:393).
- `(import-vars …)` records re-exports; a qualified `x/defn` head is a tentative definition settled after extraction (`settle_defn_macros`, clojure.rs:989).
- Every other list whose head is a `sym_lit` that does not start with `:` becomes a bare `RefRecord` (`record_call`, clojure.rs:710). The whole symbol text, `alias/f` included, goes into `name`.
- When the head is a framework registrar verb (Compojure), the plugin also captures the route string and handler arguments, and it synthesizes `handler_at_<line>` closures for handlers written in place (clojure.rs:758-962).

The plugin overrides neither `max_line_bytes` nor `ts_language_for`. It reads `ExtractCtx` for the registrar gate and for `reference_sites` (value references, §03). It builds its own locals table (`binding_spans`, §07). Resolution goes through the generic pipeline with Clojure-specific branches in `cross_file.rs` (the `require`/`refer`/`refer-all` import arms near cross_file.rs:1958-2020, the `alias/f` exact lookup and re-export following near cross_file.rs:6322 and 6780-6815) and `intra_file.rs` (`clojure_letfn_visible`, intra_file.rs:991).

Fixtures cited below live under `<scratch>/parity/clojure/` (p1 … p5). Each was run with `target/release/cgg` 0.11.0 using `-t json -o pN.json`, optionally with `--include-external --include-stdlib`. The results were dumped with `show.py` in the same directory. `dc.json` is the `--dead-code --dead-code-format json` report for p1. Claims marked 0.13.3 were re-checked on new fixtures under `<scratch>/lang-docs/clojure/` (p1, p2), run with `target/release/cgg` 0.13.3 (`-t json`, `--include-external --include-stdlib`, `--reference-edges --dynamic-dispatch`, `--dead-code`).

## 01. Detection & parsing

- **Extensions `.clj`, `.cljs`, `.cljc`, `.edn`** (clojure.rs:15-17; test `plugin_loads`, clojure.rs:507-512). p1 reports `detected_via: extension:.clj` / `.cljs` / `.cljc`. `deps.edn` → `extension:.edn` with 0 callables, which shows that data files are parsed as code.
- **No shebangs** (clojure.rs:18-20). Babashka scripts (`#!/usr/bin/env bb`, extensionless) are skipped as `unknown-extension` (p1 `script`, audit `file_skipped`). `.bb` is not a registered extension.
- **No content detection.** Clojure has no entry in `cgg-lang/src/detect.rs`: `grep clj/edn` finds nothing there.
- **One grammar for every dialect.** `.cljs` and `.cljc` use the same grammar, and there is no `ts_language_for` override. Reader conditionals `#?(:clj (a) :cljs (b))` are walked as plain children, so **both** branches produce edges (p1: `app.shared/plat -> inner-clj` and `-> inner-cljs`, both `high`).
- **Reader conditionals in `ns`.** `#?(:clj [a.b :as x])` and `#?@(:clj ([a :as x]))` require entries are read (`reader_branches`, clojure.rs:1051; test `reader_conditional_requires_are_recorded`). Fixture `<scratch>/lang-docs/clojure/p1` (0.13.3): `(o2/z)` through `#?(:clj [app.other :as o2])` binds `app.other/z` `high`.
- **No re-parse or fallback on parse errors.** The tree-sitter ERROR tree is walked as-is (test `unbalanced_source_does_not_panic`).
- **Skip conditions.** Only the generic ones apply:
  - `MAX_TREE_DEPTH` = 4,000 → `SkipReason::TooDeep` (crates/cgg/src/lib.rs:155, 451-454).
  - No `max_line_bytes` limit (default `None`, lib.rs:313).
  - Value references are not recorded in files over `cgg_lang::MINIFIED_LINE_BYTES` (CLAUDE.md).
- **Test-file classification**: the generic directory rule, plus a Clojure file-name arm `_test.clj`/`_test.cljs`/`_test.cljc` (cgg-core/src/testfile.rs:152-153). The 0.13.3 fixture's `test/app/core_test.clj` `defn test-helper` gets `test_role: support`.

## 02. Callables extracted

- **13 `def*` heads become callables** (`is_defn_form`, clojure.rs:101-123): `defn`, `defn-`, `defmacro`, `defmacro-`, `def`, `defonce`, `defprotocol`, `definterface`, `deftype`, `defrecord`, `defstruct`, `defmulti`, `defmethod`. The test `every_def_form_is_recognised` (clojure.rs:525-550) checks 12 of the 13; `defmacro-` is not exercised.
  - All of them get `DefVariant::FreeFunction`, so JSON `kind` is `function`. `defmacro`/`defmacro-` carry the `macro` attribute, `defmulti` the `multimethod:<qn>` attribute. A `def` of a non-function (`(def ^:dynamic *dyn* 1)`, `(def handler-val inner)`) is still a "callable" (p1).
  - The span covers the whole form (test `definition_spans_cover_the_whole_form`, clojure.rs:645-661).
  - `signature_hint` = `extract_signature(form text)` (plugins.rs:74). For Clojure that is usually the entire form, because no `{` appears at depth 0. Example from p1: `"(defn run [] (u/helper 1) …)"`.
- **Qualified name** = `<ns>/<name>` when an `ns` form came earlier in the file, otherwise the bare name (`qn`, clojure.rs:57-63). Tests: `the_namespace_qualifies_every_definition`, `without_a_namespace_the_bare_name_is_used`.
  - `ns` metadata (`(ns ^{:added "1.0"} a.b)`) is stripped through the grammar's `name` field (test `namespace_metadata_is_not_part_of_the_name`).
  - **Name metadata is not part of the name.** `(defn ^:private hidden …)`, `^String typed`, `^{…}`, `#^Tag`, stacked metadata and form-level metadata (`^:x (defn k …)`) are skipped (`sym_text`; test `name_metadata_is_not_part_of_any_definition_name`). 0.13.3 fixture p1: `app.util/hidden`, `app.util/typed`.
  - Only the most recent `ns` form applies, because `self.namespace` is overwritten. Two `ns` forms in one file therefore re-qualify everything after the second one (unverified by fixture; follows from clojure.rs:131).
- **Owners.** `defn`-family definitions have no owner; the namespace acts as the "enclosing owner".
- **Protocol / interface declarations.** Each `defprotocol`/`definterface` signature is an abstract method `ns/m` (kind `method`, attributes `protocol:<P>` and `abstract`). 0.13.3 fixture p1: `(defprotocol Speaker (speak [this]) (shout [this msg]))` yields `app.core/Speaker`, `app.core/speak`, `app.core/shout`.
- **Type and extension methods.** A method list in `deftype`/`defrecord`/`extend-type`/`extend-protocol` is a method `ns/Type.m` (simple name `Type.m`, `protocol:<P>` attribute) owning the calls in its body; `reify`/`proxy` methods are `ns/P.m`. p1: `app.core/Dog.speak`, `app.core/Dog.shout`, `app.core/String.speak` (from `extend-protocol Speaker String …`), `a.m/Runnable.run` (from `reify`, p2).
- **Multimethods.** Each `defmethod` is `ns/m[dispatch]` (dispatch text, max 60 chars) carrying `multimethod:<defmulti qn>` resolved through the file's aliases (test `defmethods_name_their_multimethod`). p1: `app.core/area`, `app.core/area[:circle]`, `app.core/area[:square]`.
- **`letfn` specs** are callables nested in the enclosing definition: `ns/outer.k`, simple name `k`, attribute `letfn` (`cgg_core::CLOJURE_LETFN_ATTR`; test `letfn_specs_are_local_functions`). p1: `app.core/run.loc`.
- **Schema-style heads.** `(s/defn f :- T [x :- T] …)`, `(mu/defn- …)`, `(mu/defmethod …)`, `(core/defn …)` are definitions when the head's namespace resolves to one in `frameworks::rules::CLOJURE_DEFN_MACRO_NAMESPACES` (`clojure.core`, `schema.core`, `malli.experimental`) or to a namespace of the tree defining or `import-vars`-re-exporting that macro (`settle_defn_macros`, run in `analyze_in_pool`; test `schema_defn_heads_are_settled_definitions`).
- **Not supported / partial:**
  - **Multi-arity `defn`** (`(defn multi ([] …) ([x] …))`) is ONE callable. Arities are not split and no arity is recorded (p1 `app.core/multi`, one node).
  - **Not definitions at all:** `extend` (the map form), `gen-class` methods, `fn` / `#(…)` lambdas, `deftest`, `defspec`, `defroutes`, `s/fdef`/`s/def` (spec), `declare`, and any user `def*` macro whose namespace is not settled as a `defn` macro (`defn-traced`, `defsc`, …). `deftest` is named explicitly as missing in the `clojure-test` rule gap (rules.rs:3590).
  - **Quoted templates.** A `defn` whose name is unquoted (`~n`, `~(symbol …)`, an `unquoting_lit`) is skipped (test `unquoted_defn_names_are_not_definitions`). A quoted `` `(defn literal-name …) `` inside a macro body would still be a definition (unverified).
  - **Synthetic route handlers.** `handler_at_<line>` is a `NamedClosure` with attribute `synthetic`, covered in §10.

## 03. Call-site forms

- **Bare call** `(inner x)` → `RefRecord{name:"inner"}` (`record_call`, clojure.rs:710; test `calls_inside_a_body_are_references`). It binds intra-file at `high` (p1 `run -> inner`).
- **Anonymous fn literal** `#(inner %)` is read like a list, so its body's calls are recorded (test `calls_inside_an_anonymous_fn_literal_are_calls`; p1 L23 `run -> inner`).
- **Hinted call** `^String (f x)` is still a call (test `a_hinted_call_is_still_a_call`).
- **Discard reader** `#_(gone)` yields nothing (`dis_expr` is skipped; test `a_discarded_form_yields_nothing`; p1 L24: no edge to `gone`).
- **Value references** (only under `--reference-edges` or `--dead-code`): symbol arguments of calls (threading steps included: `(-> 1 inner)`), `#'f`/`#'ns/f`, `def` values, binding-vector inits, `fn`/`defn` body forms, and map values and vector elements holding one are `RefRecord::value_ref` records. A shadow filter reads the locals table; quoted data, `case` constants and `ns`/`comment`/`declare` arguments are not values (`CLOJURE_NON_VALUE_FORMS`); bare names are kept only when the file defines or `:refer`s them (`keep_reachable_values`, clojure.rs:1090). 0.13.3 p1 with `--reference-edges`: `(map inner [1])` and `(-> 1 inner)` give `run -> inner` `reference` `high`; `(def handler-val inner)` gives `handler-val -> inner`; `(map u/unused-pub [1])` gives `run -> app.util/unused-pub` cross-file `high`.
- **Namespace-qualified / alias call** `(u/helper 1)` and `(app.util/helper 2)`. The whole symbol is the name with an empty receiver (clojure.rs:251-269), and the cross-file Clojure branch splits it on `/` (§04). p1: both → `app.util/helper` `high` `cross-file:imports`.
- **Keyword-as-function** `(:k m)` is never recorded (clojure.rs:252; test `keywords_in_call_position_are_not_references`).
- **Calls inside special forms** are recorded because the walker recurses into every `list_lit`: `->`, `->>`, `when-let`, `let`, `fn` and `letfn` bodies (p1, p4).
  - The special-form heads themselves (`let`, `fn`, `->>`, `when-let`) are recorded as calls and classified as `<stdlib>` (p1 with `--include-stdlib`).
- **Not supported / partial:**
  - **Threading-macro bare symbols** are value references, not calls: `(-> x dup)` gives no `direct` edge in the default graph.
  - **Java interop.** All of these are recorded with the literal symbol as the name and end up `<external>` (p1 L36-39, L48-49; p4 L12): `(.toUpperCase s)` → name `.toUpperCase`; `(Math/abs x)` → `Math/abs`; `(String. "x")` / `(Date.)` → `String.` / `Date.`; `(->Dog …)` → `->Dog`. There is no receiver typing, and the record constructor `->Dog` is not linked to `defrecord Dog`.
  - **ClojureScript interop** `(js/console.log …)` and `(.log js/console …)` → external (p1 front.cljs).
  - **`(comment (inner 1))` still records the call:** 0.13.3 p1 L25 `run -> inner`, `high`, a false edge. `comment` itself is external because it is not in the stdlib list.
  - **No arity is recorded** (`arity: None` in every record), so there is no overload narrowing (§06).
  - **No explicit generics or operator overloading.** Not applicable to Clojure.
  - **No paren-less calls.** Not applicable.

## 04. Imports & module resolution

- **`(ns name (:require …) (:use …) (:import …))`** is read only inside the `ns` form (`extract_namespace`, clojure.rs:238-267), reader-conditional branches included. Each entry becomes an `ImportRecord` with kind `require` / `use` / `import` (`record_require_entry`, clojure.rs:291-391; test `use_and_import_keep_their_own_kinds`).
  - `sym_lit` entry `a.b` → path `a.b`, no alias.
  - `vec_lit` entry `[a.b :as x]` → path is the first symbol, alias is the symbol after `:as` (tests `require_with_an_alias_records_path_and_alias`, `a_bare_require_vector_has_no_alias`).
  - `:refer [f g]` adds one `refer` record per name; `:refer :all` adds a `refer-all` record; `:use` refers the namespace whole.
- **`import-vars` re-exports.** `(p/import-vars [a.b f])` in `x` records a re-export (`cgg_core::CLOJURE_REEXPORT_KIND`, `record_reexports`, clojure.rs:477), so `x/f` and `#'x/f` reach `a.b/f`.
- **How a `require` resolves** (cross_file.rs:1846-1892):
  1. Every definition of a file found by trying the paths `p`, `p.clj` and `p` with `.`→`/` + `.clj` (plus the Ruby/Lua variants) becomes reachable through `collect_include_defs` (budget 4).
  2. The alias, or else the namespace's last dotted segment, is put in `module_aliases`.
  3. The path is pushed to `unqualified_prefixes`.
- **`alias/f` and `full.ns/f`** map the alias through the file's `require` aliases to the exact qn `ns/f` (following `import-vars` re-exports, indexed into `by_qn`).
  - When found, it binds `high` (test `a_clojure_alias_call_binds_in_the_required_namespace`). 0.13.3 p1: `(u/helper 1)` and `(app.util/helper 2)` → `app.util/helper` `high` `cross-file:imports`.
  - When not found the site never binds by name elsewhere and goes `<external>`.
- **Bare calls are scoped.** A bare name reaches only its own namespace (intra-file), a `:refer`'d var (`refer` records → `direct_imports`, `high`) or a namespace referred whole (`refer-all`/`:use`); never by-name fan-out. `require` registers only the alias. 0.13.3 p1: `(w)` with `:refer [w]` → `app.util/w` `high`; `(hidden)` and `(y)` with no refer → `candidates-in-other-files`, unbound. p2: `[my-lib.core :refer [hy]]` → `(hy)` binds only `my-lib.core/hy` `high`, no longer the unrelated `app.other/hy`.
- **Not supported / partial:**
  - **Prefix lists** `(:require (a [c :as cc]))` and `[a [c :as cc]]` are not expanded: `cc/z` → external, and bare `(z)` → `candidates-in-other-files` (p3 `a.q`).
  - **`:as-alias`** is not recognised as an alias (`seen_as` only for `:as`). 0.13.3 p2: `ca/z` → external.
  - **Top-level `(require '[a.c :as k])`** / `(require (quote …))` outside `ns` → not an import. 0.13.3 p2: `k/z` → external. The same applies to `use`, `import`, `refer`, `load` and `alias` calls.
  - **`:rename` / `:exclude` / `:only`** are not read.
  - **`(:import [java.util Date])`** is recorded (kind `import`, path `java.util`). Nothing consumes it for resolution.
  - **`FileFacts::packages` is not populated.** Clojure has no `packages`, and the namespace lives only in qns.
  - **No manifest reading.** `deps.edn`, `project.clj` and `shadow-cljs.edn` are parsed as Clojure code, not read as manifests (no Clojure branch in `plugins/manifest.rs`).
  - **No `heads.rs` external-module refusal.** Clojure is not in WorkspaceVocab, which covers Python/JS/TS/Go/Java/Kotlin/C#/C++ (CLAUDE.md).
  - **No `modules.rs` `not-in-imported-module`.** `modules.rs:190` lists js/ts/python/go/lua only. The closest equivalent is the alias branch's silent `None`.
- **Stdlib alias tables are keyed by conventional alias, not by namespace.** `stdlib/clojure.txt` lists `str/join`, `io/file` and `walk/…`. Since 0.11.1 a fully qualified `clojure.string/join` is stdlib through `STDLIB_MODULES` (cgg-core/src/stdlib.rs:158-184). 0.13.3 p2 results:
  - `(s/join …)` under `[clojure.string :as s]` → still external (the alias is not followed);
  - `(clojure.string/join …)` → `<stdlib>`.
- **Bug (still present at 0.13.3): a resolved alias call also mints an `<external>` exit node.** With `--include-external`, p1 `(u/helper 1)` produces both `run -> app.util/helper high` and `run -> <external>::u/helper low` (also `app.util/helper`, `o2/z`). The audit reconciliation does not prune the external bucket entry for sites the Clojure branch bound.

## 05. Type inference & receiver typing

- Not applicable / none. There is no Clojure code in `type_hints.rs`, `receiver.rs` or `closure_types.rs` (`grep clojure` finds nothing).
  - Type hints (`^String x`, `^T t`) are not read (a hint on the def name is stripped from the name since 0.11.1, §02).
  - `(.bar t)` with `^T t` → `<external>::.bar` (p4).
  - Locals and parameters are recorded only for shadowing (§07), never typed. There are no casts, constructions, wrappers, implicit receivers or weak receiver types.

## 06. Inheritance, dispatch & overloading

- **Base / implementation records.** No `base_types`, `trait_impl_target` or `ClassDecl`; the link is the `protocol:<P>` / `multimethod:<qn>` attribute.
  - The dead-code capability table still shows `dispatch none` (0.13.3 p1 report).
- **Protocol dispatch.** A call to the protocol var `(speak x)` binds the abstract declaration `app.core/speak` (`high`, 0.13.3 p1 L31), never an implementation directly. `dispatch::fanout` (`--dynamic-dispatch`, forced on by `--dead-code`) adds `Via::Dynamic` `low` edges from each declaration to its implementations by the `protocol:` attribute: p1 `speak -> Dog.speak`, `speak -> String.speak`, `shout -> Dog.shout`.
- **Multimethods.** A call to `(area x)` binds the `defmulti` only (`high`, p1 L32); `--dynamic-dispatch` links `area -> area[:circle]` and `area -> area[:square]` (`dispatch:fanout`, `low`). The dispatch value is not used to narrow.
- **Not a member of:** `INHERITANCE_LANGS`, `OVERLOAD_ARITY_LANGS`, the clause languages (erlang/elixir/haskell), or `member_of_calling_class` / `overrides_below`.
- **No `super`/base calls, extension methods, mixins or arity narrowing.** Multi-arity `defn` is one node, and the self-call `(multi 1)` binds `multi -> multi` `high` (p1 L27).
- **Redefinition** (`(defn dup …)` twice) → `ambiguous-in-file`, unresolved. Clojure's last-def-wins rule is not modelled (p4 `use-dup`, `->thread`, `T`, `hof`).

## 07. Scoping & shadowing

- **`FileFacts::locals` from the plugin's own table.** `plugins::clojure::binding_spans` is built once and recorded as the file's locals; `locals.rs` skips its own collection for Clojure (locals.rs:554). It covers `fn`/`defn`/`defn-`/`defmacro`/`defmethod` parameters over their arity's body (`&`, vector and map destructuring incl. `:keys`/`:strs`/`:syms`/`::keys`/`:ns/keys`/`:as`; never an `:or` default or a schema after `:-`), the `fn` self-name, method parameters and type fields, `letfn` names, `let`/`loop`/`doseq`/`for`/`when-let`/… bindings from the end of their init, other `with-…` vectors and `catch`. `binding`/`with-redefs`/`…-redefs` rebind vars, not locals (`stdlib::CLOJURE_VAR_REBINDING_FORMS`/`_SUFFIXES`).
  - 0.13.3 p1: `(let [inner (fn [] 0)] (inner))` is `shadowed-by-local` (L29), no longer a wrong `high` edge.
- **Lexical narrowing: `letfn` only.** A `letfn` function is visible where the form's binding of the name covers the site (`intra_file::clojure_letfn_visible`, intra_file.rs:991). p1: `run -> run.loc` `high`, `run.loc -> inner` `high`.
- **Positional shadowing: no.** That list is ocaml/fsharp/lua only. A later `(defn dup …)` does not shadow an earlier one (§06; 0.13.3 p2: `(dup)` with two `defn dup` is `ambiguous-in-file`).
- **`bare_call_is_never_a_method`: not listed** (intra_file.rs:49-54). This has no effect, because Clojure defs carry no owners.
- **No conditional definitions.** `(when x (defn …))` would still be extracted, and `DefRecord::conditional` is never set (unverified by fixture; the walker extracts any matching list at any depth, clojure.rs:65-79).
- **Forward references** through `declare` work, because resolution is file-wide: `(declare later)` + `calls-later -> later` `high` (p1). `declare` itself becomes a ref.

## 08. Macros, preprocessor & codegen

- **`defmacro` / `defmacro-` are ordinary callables** carrying the `macro` attribute. Calls to them bind like functions, and calls inside the macro's syntax-quoted template are attributed to the macro. Inside a syntax-quote a macro's locals are in scope only under `~`.
- **Macro expansion: none.** Code a macro generates is invisible, and nothing is tagged `macro:<name>`. Clojure is not in the `rust_macro`/`c_macro` paths.
- **Quote / unquote.** `defn ~n` and `(defmulti ^:private ~n …)` templates are skipped (§02).
- **Reader conditionals** `#?` / `#?@`: both branches are walked (§01). The discard reader `#_` is honoured (§03).
- **`name_mentions`: never populated** by the Clojure plugin.
- **Preprocessor / `#define` / `#if` / templates:** not applicable.

## 09. FFI & cross-language links

- **None.** `ffi.rs` has no Clojure handling. Java interop calls, `gen-class` exports and ClojureScript `js/` calls all go `<external>`. `^:export` metadata is stripped from the name but is not an attribute or a root.
- **Descriptor links** (protobuf rpc → host): no Clojure host binding.
- **Markup code-behind:** not applicable.

## 10. Frameworks & entry points

Six rules carry `language: "clojure"` (rules.rs:3482-3592). No multi-language rule names Clojure.

- **`compojure`**, kind `Network`, `detect: ["compojure"]`, registrars `get post put patch delete head options any context defroutes` (rules.rs:3482).
  - Since 0.11.1 a registrar verb that is also a `clojure.core` function (`get`), written without a route string, captures nothing (test `a_core_get_without_a_route_captures_no_handler`). This is the only enumerating rule. `app-metabase-compojure` in benchmark.sh:200 is the verified app.
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

- **`clojure-test` is a detect-only gap rule** (rules.rs:3578-3592). `deftest`, `testing` and `defspec` are not definitions, so test bodies have no callable. 0.13.3 p1 `core_test.clj`: `a-test` absent, and `(c/inner)` inside it has no enclosing callable, so it makes `app.core/inner` a `toplevel:invocation` dead-code root instead of an edge.
- **Name convention.** `is_test_name` falls to the default arm (`test_` / `Test` prefix, deadcode/roots.rs:975-988). An underscore name like `test_thing` becomes a `builtin:test-name` root (p1 dc.json). Idiomatic kebab names (`test-helper`, `foo-test`) do not match.
- **No test harness node** (`<framework-entry>::test::…`). Clojure is not in `TEST_CLASS_MEMBERS_ARE_ROOTS`, and there is no `TEST_ATTRS` for it.
- **Test-file gating**: the `test/` directory rule and the `_test.clj(s/c)` file-name arm (§01).
- **Missing:** kaocha, midje (`fact`), test.check (`defspec`), expectations, and cljs.test `async`.

## 12. Dead-code analysis

- **Language class `degraded`.** At 0.13.3 the capability row reads `vis no, attrs yes, refs yes, disp no` (p1 dead-code report: `visibility none`, `attributes full`, `value_references full`, `dispatch none`, `max_confidence medium`), so every Clojure finding is still capped at **medium**, with evidence `language-lacks-visibility` and `language-lacks-dispatch-model`.
- **Visibility is not extracted.** `defn-` and `^:private` are not recognised, and `visibility` is `""`. `private-one` gets the same evidence as public functions. There is no `PrivateVisibility` corroboration.
- **Roots that fire for Clojure** (dc.json):
  - `builtin:main` for a fn named `main` (UNIVERSAL_ENTRY, roots.rs:19).
  - `builtin:test-name` for `test_*` / `Test*`.
  - `toplevel:invocation` for any name called outside every callable; a top-level `(alias/f)` or `#'alias/f` roots exactly that var, through aliases and re-exports. 0.13.3 p1: `inner` is a root through the `deftest` body.
  - `framework:entry-node` for Compojure entries.
  - `clojure:external-contract` for a method implementing a protocol or interface the tree does not declare, and a `defmethod` of a multimethod the tree does not declare. 0.13.3 p2: `a.m/Runnable.run` (`reify Runnable`) and `a.m/print-method[String]` are roots.
- **`-main` is not a root.** Clojure's `:gen-class` entry `-main` is reported as a `medium` finding (0.13.3 p1 `app.core/-main`). `LANG_ENTRY` has no Clojure row, and `LIFECYCLE` has none either.
- **Not roots:** `^:export` (cljs) functions and `deftest`s (they are not callables). Protocol implementations and `defmethod`s of in-tree contracts are reached through the dispatch fan-out, and values (`(def handler-val inner)`, `(map f xs)`) are reference edges under `--dead-code`.
- **Exports: none** (`exports: false`), so there is no `ExportedApi` root.
- **Confidence caps.** `implements-external-contract` is never applied (outside contracts are roots instead, above). `macro-generated` never applies (no `macro:` tags). The `reflection` cap never applies (no dynuse, §15).
- **Unresolved-doubt handling** is generic: p1 findings whose name matches an unresolved bare call (`hidden`, `y`) carry `name-matches-unresolved-site`.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/clojure.txt`, 367 lines**, loaded at stdlib.rs:211 and registered at stdlib.rs:252. It holds clojure.core names, including special forms and macros (`fn`, `let`, `letfn`, `->`, `->>`, `defn`, `deftest`, `reify`, `extend-protocol`, `proxy`, `ns`, `require`, `resolve`, `ns-resolve`, `load-file`).
  - It also holds alias-prefixed entries under conventional aliases only: `str/…` (about 20), `io/…` (9) and `walk/…` (4) (clojure.txt:143-151, 287-309, 352-355).
  - Missing: `eval`, `requiring-resolve` and `comment` (0.13.3 p1: `comment` → external).
- **How a name is classified.** The classifier splits a call's own qualifier on `::`, `/` and `\` and checks `STDLIB_MODULES`, which lists the `clojure.*` namespaces (stdlib.rs:158-184). So `clojure.string/join` is stdlib, `str/join` matches as a literal entry, and `s/join` (alias) is external (0.13.3 p2).
- **No CAST_INTRINSICS, CONSTRUCTING_METHODS, implicit-package or rust_types-like tables** for Clojure.
- **`--include-stdlib`** mints `<stdlib>::<name>` leaf nodes (p1: `->>`, `map`, `filter`, `fn`, `let`, `letfn`, `resolve`, `str/join`). `--include-external` mints `<external>::…` for everything else, including the duplicate exit node for resolved alias calls (§04).

## 14. Confidence & certainty

- **`high` comes from** (0.13.3 p1):
  - Intra-file single-candidate bare calls (`run -> inner`, `-main -> run`), including a protocol var call to its declaration and a multimethod call to its `defmulti`.
  - Alias and fully-qualified calls (`u/helper`, `app.util/helper`) and `:refer`'d bare calls (`w`).
- **`low`** is used for framework-entry edges, dispatch fan-out and exit nodes.
- **False `high` edges still observed** at 0.13.3:
  - `(comment (inner 1))` (p1 L25);
  - the both-branches `#?` edges (from the 0.11.0 audit; not re-run).
- The 0.11.0 false-`high` shapes for `#_`, `let` shadowing and un-referred bare calls are fixed (§03, §04, §07).
- **Mermaid** prefixes `?` on arrows with no `high` site.
- **No `CertainCtx` / `OwnerFacts` paths** apply, because there are no typed receivers.

## 15. Dynamic use & reflection

- **None.** The plugin never calls `plugins/dynuse.rs` (no `clojure` match in dynuse.rs at 0.13.3). In p1:
  - `(resolve 'app.util/unused-pub)` → only a stdlib `resolve` edge. The quoted symbol is not a value (quoted data is skipped) and does not mark the var live.
  - `requiring-resolve` → external (0.11.0 audit).
- **`ns-resolve`, `find-var`, `var-get`, `#'var`, `eval`, `load-file`, `load-string` and `(apply (symbol …))`**: no screen and no cap. `DYN_MODULE_LOADERS` is `["require", "importScripts", "import_module"]` (cgg-core/src/facts.rs:165), but it applies only through dynuse, which Clojure does not use.

## 16. Known gaps & limitations

- **Framework gap strings** (rules.rs): ring, http-kit, pedestal, reitit, clojure-test (rules.rs:3590: "the Clojure plugin does not treat `deftest` as a definition at all").
- **Fixed since the 0.11.0 audit** (re-checked with 0.13.3 fixtures p1/p2): name metadata in callable names; protocol/interface/type/`reify`/`extend-*` members (now methods, with dispatch fan-out); value references; the locals table; `#_` discards; `#(…)` bodies; `letfn` functions; multimethods sharing one qn; `:refer` scoping and the wrong hyphenated-namespace fan-out.
- **Gaps and bugs still confirmed at 0.13.3:**
  1. `defn-` / `^:private` visibility is not recorded, and there is no dispatch capability, so the language is `degraded` and every dead-code finding is capped at medium.
  2. `(comment …)` bodies produce false edges.
  3. Not understood: prefix lists, `:as-alias`, top-level `(require …)`, and stdlib aliases (`s/join`).
  4. With `--include-external`, a resolved alias call also emits an `<external>` exit edge.
  5. `deftest`/`defspec` are not callables, and `-main` and `^:export` are not roots.
  6. Redefinitions are `ambiguous-in-file`, with no last-def-wins.
  7. No reflection screening for `resolve` / `requiring-resolve` / `ns-resolve`.
  8. `.edn` data files are parsed as code. `.bb` files and the `bb` shebang are not detected.
  9. The Compojure `context` + nested verb on one line mint two `handler_at_<line>` callables with one name (`value-ref-ambiguous`; from the 0.11.0 audit, not re-run).
  10. Multi-arity `defn` is one callable with no arity.
- **README** language table row: `| Clojure | (ns :require ...) | — | defn/defmacro/deftype/defprotocol |` (README.md:513). The benchmark repo is `clojure-ring` (`ring-core/src`, benchmark.sh:87): 209 callables, 212 edges, 8% cross-file (README.md:1339).
- **CHANGELOG follow-ups.** The perf note says Metabase's Clojure `require` graph made path normalisation cost 1.1 s (cross_file.rs:3426-3433). No open Clojure follow-up items were found in CHANGELOG.md beyond the fixed alias and metadata entries (CHANGELOG.md:233-235).
- **Ignored tests:** none. `grep '#\[ignore' clojure.rs` finds nothing.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | .clj/.cljs/.cljc/.edn by extension; no shebang/`.bb`; `#?` both branches walked; `.edn` data parsed as code |
| 02. Callables extracted | good | 13 `def*` heads as `ns/name` (metadata stripped); protocol/interface declarations, type/`reify`/`extend-*` methods, `defmethod` `m[dispatch]`, `letfn` specs, schema `defn`s; no lambdas, deftest, multi-arity split |
| 03. Call-site forms | good | list-head and `#()` calls incl. `alias/f`; `#_` skipped; value refs under `--reference-edges`/`--dead-code`; `comment` bodies still produce fake calls |
| 04. Imports & module resolution | partial | `ns` `:require/:use/:import` (reader conditionals too) with `:as`, `:refer`, `import-vars`; exact `alias/f` (high); bare calls scoped; no prefix lists, `:as-alias`, top-level require |
| 05. Type inference & receiver typing | none | type hints ignored; locals recorded only for shadowing |
| 06. Inheritance, dispatch & overloading | partial | protocol and multimethod dispatch fan-out by attribute (`--dynamic-dispatch`); no arity narrowing |
| 07. Scoping & shadowing | partial | plugin-built locals table (`binding_spans`); `letfn` lexical visibility; no positional shadowing |
| 08. Macros, preprocessor & codegen | minimal | defmacro is a callable (`macro` attr); `~name` templates skipped; no expansion, no `macro:` tags, no name_mentions |
| 09. FFI & cross-language links | none | Java/JS interop all external; no ffi or descriptor links |
| 10. Frameworks & entry points | partial | compojure enumerates (registrar capture + inline handler synthesis); ring/http-kit/pedestal/reitit are detect-only gaps |
| 11. Test harness entries | none | clojure-test is a gap rule; deftest not a callable; only `test_`/`Test` name roots |
| 12. Dead-code analysis | partial | class degraded (no visibility, no dispatch capability), all findings medium; external-contract and top-level alias roots; `-main`/`^:export` not roots |
| 13. Builtins & stdlib tables | partial | 367-name clojure.txt incl. special forms; alias entries keyed by conventional alias only (`str/`, `io/`, `walk/`) |
| 14. Confidence & certainty | partial | high for intra-file unique, exact alias and `:refer` calls; `comment` bodies still give false highs |
| 15. Dynamic use & reflection | none | no dynuse; resolve/requiring-resolve/ns-resolve unscreened |
| 16. Known gaps & limitations | n/a | 10 gaps confirmed at 0.13.3; 5 framework gap strings; no ignored tests |
