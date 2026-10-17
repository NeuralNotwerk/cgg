# javascript

The JavaScript plugin (`JavaScriptPlugin`, `crates/cgg-lang/src/plugins/javascript.rs`, 1,621 lines including inline tests) uses the crates.io grammar `tree-sitter-javascript` (`tree_sitter_javascript::LANGUAGE`, javascript.rs:45-47); nothing is vendored. A single recursive walker, `JsWalker` (javascript.rs:118-1340), records definitions, call sites, ESM/CommonJS imports, re-exports and local exports. Its scope stack holds only class and interface names, never functions or object literals. The walker is `pub(crate)` and is reused unchanged by the TypeScript plugin (`typescript.rs:12,76`). That is why the walker has branches for TypeScript-only node kinds (`interface_declaration`, `abstract_class_declaration`, `method_signature`, `import_require_clause`) that never fire on JavaScript input. The plugin declares the signals `unreachable`, `attributes`, `impls` and `value_refs` (javascript.rs:35-43). It does **not** declare `visibility`, `exports`, `test_defs` or `dyn_uses`. No resolver kind is declared, and resolution is handled by the generic pipeline. JS has JS/TS-specific branches in `cgg-resolve`:

- `modules.rs`: `ModuleIndex`/`FileModules`, which follows imports to files.
- `heads.rs`: npm-package external-head refusal.
- `cross_file.rs`: step 0c, `bare_qns`, the `constructor` mapping and `in_caller_file` anchoring.
- `intra_file.rs`: lexical scoping gated on locals.

Fixtures cited below live under `<scratch>/parity/javascript/` (f1 … f11). They were run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, and dumped with `show.py` in the same directory.

## 01. Detection & parsing

- **Extensions `.js`, `.mjs`, `.cjs`, `.jsx`.** Evidence: javascript.rs:29-31; detect.rs:681-682 test table (`c.js`, `d.mjs`); `crates/cgg/tests/detect.rs:70` (`j.js`). Fixture f6: `x.cjs` → `extension:.cjs` and `x.mjs` → `extension:.mjs`.
- **`.jsx` is parsed with the plain JavaScript grammar**, which accepts JSX (fixture f3 `comp.jsx` parsed `ok`). There is no `ts_language_for` override and no separate JSX grammar.
- **Shebang `node`**: `#!/usr/bin/env node` with no extension is detected as `javascript` (`shebang:node`). Evidence: javascript.rs:32-34; test `node_shebang_maps_to_javascript` (detect.rs:720).
  - Fixture f6: `#!/usr/bin/env nodejs` and `#!/usr/bin/env -S node --experimental` are both also detected as `shebang:node`, so the shebang match is by prefix.
  - **Not JS**: `#!/usr/bin/env bun` and `#!/usr/bin/env deno` are detected as **typescript** (fixture f6, `shebang:bun`, `shebang:deno`).
- **Unsupported extensions**: `.es6` is skipped as `unknown-extension` (fixture f6). `.vue`, `.svelte` and `.astro` are not parsed at all; this is stated in the `vue`/`svelte`/`astro` rule gaps (rules.rs:11574, 11608, 1921).
- **Content detection**: none for JS. Content detection exists only for OpenAPI/AsyncAPI YAML/JSON and C/C++ headers.
- **Re-parse/fallback**: none. A file with syntax errors is still extracted with `parse_status = "error"` (cgg/src/lib.rs:457-461). **Partial**: error recovery can misclassify definitions. Fixture f6: in `broken.js` (`function broken( { return 1;` followed by `function ok(){}`), `ok` was extracted with kind `inherentmethod`.
- **Depth skip**: a tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep` (cgg/src/lib.rs:155, 451-456). This is shared by every language.
- **Minified handling** has three layers:
  1. `--skip-minified` (off by default) skips `*.min.{js,mjs,cjs}` and any `.js/.mjs/.cjs` file averaging over 2,000 bytes per line (`MINIFIABLE_EXTS`, cgg-walk/src/lib.rs:72-81; test `minified_js_is_skipped_by_name_and_by_line_length`, cgg-walk lib.rs:626). Fixture f11 with `--skip-minified`: `1 minified` skipped.
  2. Without the flag, a file with any line over 2,000 bytes gets no `FileFacts::locals` (`MINIFIED_LINE_BYTES`, cgg/src/lib.rs:159, 478-486). Lexical narrowing is then also off for that file (intra_file.rs:363-366).
  3. `CertainCtx::minified_caller` (cross_file.rs:4545-4550) means a minified calling file (average line over 1,000 bytes) never gets a **cross-file** `high`. Fixture f11: *intra-file* edges in a 6 KB single-line bundle were still `high` (`a -> b`, `c -> K.m`).
- **Line-length cap**: none. `max_line_bytes` is not overridden (default `None`, cgg-lang lib.rs:313-315).
- **Manifest reading during extraction** (`manifest::js_module_manifest`, manifest.rs:231-290):
  - The nearest named `package.json`.
  - Every `deno.json`/`deno.jsonc`/`jsr.json` above the file, with their local `imports` keys.
  - `compilerOptions.paths` keys of the nearest `tsconfig.json` **or `jsconfig.json`**, following `extends`.
  - The `exports` map of the owning package.
  - The results go to `FileFacts::packages` and to `package-name`/`package-export` import records (javascript.rs:59-66, 96-116).

## 02. Callables extracted

Qualified names are the class/interface scope joined with `.` (`JsWalker::qn`, javascript.rs:151-157). **There is no module or file prefix.** JS QNs are bare: `plain`, `Base.run`. `cross_file.rs:5605-5608` calls this out as `bare_qns` ("every lookup of one below by exact name is a lookup by bare name").

- **Function declaration** `function plain(a){}` → `FreeFunction`, QN `plain` (record_function, javascript.rs:283-306). Test `function_declarations` (javascript.rs:1504). Fixture f1.
- **Generator declaration** `function* gen(){}` → `FreeFunction` (javascript.rs:161). Fixture f1 `gen`.
- **Async function** `async function asyncFn(){}` → `FreeFunction`. There is no `AsyncFunction` variant for JS, unlike Rust. Fixture f1 `asyncFn` kind `function`.
- **Named arrow or function expression in a declaration**, e.g. `const arrow = (x) => …`, `let fexpr = function(){}`, `const genExpr = function*(){}`, `var named = function inner(){}`. These become `FreeFunction` named after the **variable**: `named`, not `inner` (try_record_named_fn, javascript.rs:472-517). Test `arrow_function_named` (javascript.rs:1517). Fixture f1: `arrow`, `fexpr`, `genExpr`, `named`.
- **Class members** (`record_method`, javascript.rs:308-356; test `class_methods` javascript.rs:1531):
  - `constructor(){}` → `Constructor`, QN `Base.constructor`.
  - Instance method → `InherentMethod` `Base.run`.
  - `static make(){}` → `StaticMethod`. It is reported with kind `method` in the JSON.
  - `get label(){}` / `set label(v){}` → `Property`. The getter and setter are **two nodes with the same QN** (fixture f1: `Base.label` twice; f9: object-literal `v` twice).
  - `#secret(){}` → QN `Base.#secret`, `Vis::Private` (test `class_member_visibility_follows_private_names`, javascript.rs:1559).
- **Class expression** `const K = class Kls { m(){} }` uses the class's own name: QN `Kls.m`. **Partial**: an anonymous class expression (`const K = class { m(){} }`) pushes no scope (javascript.rs:174-182), so its members get bare QNs. Fixture f4: `Kls.m`.
- **Object-literal methods** `const obj = { m(){} }` → a `method_definition` with **no owner**: QN `m`, kind `method`, `Vis::Unknown` (fixture f1 `m`; test javascript.rs:1559-1578).
- **Object-literal pair with a *named* function** `{ r: function rr(){} }` → `FreeFunction` named after the function (`rr`), not the key (extract_named_fn_child, javascript.rs:1115-1149). Fixture f1.
- **Named function returned** `return function name(){}` → `FreeFunction` (javascript.rs:260-264).
- **Named function passed as an argument** `defineGetter(obj,'x',function name(){})` → `FreeFunction` (extract_named_fn_args, javascript.rs:1151-1189). The README row "defineGetter" refers to this.
  - **Partial**: when the call is also a registrar call, both this node and a synthetic `handler_at_<line>` closure are minted over the same span. Fixture f3: `process.on('exit', function onExit(){})` produced `onExit` *and* `handler_at_7`, and the entry points at `handler_at_7`. `onExit` was then reported dead (f3 dead-code JSON).
- **Named IIFE** `(function iife(){ … })()` is extracted (fixture f4 `iife`).
- **Member assignments** of a function, including chained ones, become `FreeFunction` named after the **property** with **no owner** (extract_assignment_fn, javascript.rs:406-470):
  - `exports.cjsA = function(){}`, `module.exports.cjsB = () => …`, `obj.assigned = function(){}`.
  - `Legacy.prototype.greet = function(){}` → QN `greet`.
  - `req.get = req.header = function header(){}` (chained).
  - Fixture f1: `cjsA`, `cjsB`, `assigned`. Fixture f4: `greet`.
  - `Legacy.prototype.shout = function shoutNamed(){}` → QN `shout`; the function's own name is dropped.
- **`module.exports = function(){}`** → a definition named `exports` (comment at javascript.rs:931-935).
- **Constructor functions** `function Legacy(name){ this.name = name }` are plain `FreeFunction`s. `new Legacy()` binds to them (fixture f4 `caller -> Legacy` high).
- **Synthetic inline handlers**: an anonymous closure passed to a **registrar-verb** call with a registration shape (route string) becomes `NamedClosure` `handler_at_<line>`, attribute `synthetic` (extract_inline_handler, javascript.rs:1268-1340). Fixture f3: `handler_at_8` for `router.post('/admin', function(req,res){…})` and `handler_at_3` for `parentPort.on('message', m => …)`.
- **Bodiless/abstract members** (`method_signature`, `abstract_method_signature` → `TraitMethod` tagged `abstract`, javascript.rs:212-221, 359-391) are TypeScript-only node kinds. They are not applicable to JavaScript.
- **Bases on members**: every method record carries the enclosing class's `extends` list in `DefRecord::base_types` (`attrs::base_types`, javascript.rs:170, 350). This is the only class record JS writes; `FileFacts::classes` is not written (grep finds no `classes.push` in javascript.rs).
- **Decorators** `@logged run(){}` are captured into `attributes` (`attrs::collect_with_preceding`, javascript.rs:303, 349). Fixture f9: `Svc.run ['@logged']`.
- **Not extracted** (fixture evidence):
  - **Class fields holding functions**: `field = () => this.run()` and `static sfield = function(){}` (`field_definition` is not handled). Fixture f1: no `Base.field` node. The calls inside such a field have no enclosing callable (`this.run` at L18 recorded with `src=None`).
  - **Anonymous object-literal values**: `{ p: function(){}, q: () => 4 }`. Fixture f1: `obj.p()`/`obj.q()` produced no edge.
  - **Anonymous default export** `export default function () {}` (fixture f4: no node).
  - **Anonymous IIFE / arrow IIFE** `(() => { cb(); })()`. Fixture f4: `cb` recorded `no-enclosing-callable`.
  - **Anonymous callbacks** that are not on a registrar verb: `describe('x', () => {…})`, `.map(x => …)`. Their calls are attributed to the enclosing named function, or to none at top level. Fixture f3: `fixtureHelper()` inside `it(…)` → `no-enclosing-callable`.
  - **Class `static { }` blocks**. Fixture f9: `Svc.init()` inside one → `no-enclosing-callable`.
  - **Destructured declarations** `const { a } = …`: only an `identifier` declarator name is read (javascript.rs:482-484).
  - **`Object.assign(X.prototype, { wave(){} })`** members are owner-less `wave` (fixture f4).
- **Overloads/clauses**: not applicable; JS has no overloading. A redefinition yields two same-QN nodes.
- **Operators**: not applicable; JS has no operator overloading.

## 03. Call-site forms

`record_call` (javascript.rs:1191-1258) handles `call_expression` and `new_expression`.

- **Bare call** `plain(1)` → name `plain`, empty receiver (test `call_expressions_captured`, javascript.rs:1597). Fixture f1 high intra-file.
- **Method call** `obj.run()` → name `run`, `receiver_hint` = the object text (fixture f1).
- **Constructor** `new Base(1)` is read from the `constructor` field and recorded as a call to `Base` (javascript.rs:1192-1201). At resolution, step 6 maps it onto `Base.constructor` (`constructor_names`, cross_file.rs:383-391). Fixture f1: `useAll -> Base.constructor` medium.
  - With no explicit constructor, the site is recorded as `class-without-explicit-init` (fixture f2 `Store`, f5 `Animal`).
  - A subclass with no constructor bound to the base's constructor (medium) and also left a `no-candidate-in-file` record (fixture f1 `new Child(2)`).
- **`new obj.Ctor()`** is recorded with name `Ctor` and receiver `obj` (same code path). Fixture f1 produced no edge (no candidate).
- **Optional call / optional member** `plain?.(1)`, `obj?.m()` both resolve (fixture f1: `plain` high at L58, `m` medium at L59).
- **Chained calls** `plain(1).toString()` record both calls (fixture f1 `plain` high). `getThing().run()` is recorded with receiver `getThing()`. It **fans out by name** to every `run` in the tree (fixture f1: `Base.run` and `Child.run`, medium).
- **Receiver shapes** (`receiver::peel`, javascript.rs:1212-1228; receiver.rs:54-135):
  - Parentheses and `await` are peeled.
  - `(new Child(3)).run()` → typed receiver `Child` (`typed_receiver = true`). Fixture f1: `Child.run` **high**.
- **Tagged template** ``tagged`hi` `` is recorded as a call (fixture f9 `useTag -> tagged` high). A call inside a template substitution `` `${plain(4)}` `` is recorded (fixture f1 L63 high).
- **`yield*`/`await` operands** are ordinary calls (fixture f9 `g -> other`, `aw -> fetchIt`).
- **Super calls** `super.step()` → receiver `super`, resolved through the base chain (section 06). A bare `super()` in a constructor produced no edge in fixture f5 (the base had no constructor).
- **Private method call** `this.#secret()` → `Base.#secret` high (fixture f1).
- **Value references / callbacks passed as values**: recorded **only** in argument position of a call whose last segment is a **registrar verb** of some JS rule (`registrar::capture`, javascript.rs:1253-1256; registrar.rs:424-498). These are `VALUE_REF_HINT` records, which become `Via::Reference` edges only with `--reference-edges`.
  - Fixture f10 with `--reference-edges`: `app.get('/x', listUsers)` → `setup -> listUsers` (medium, `reference`), and `app.on('evt', mapper)` → `setup -> mapper`.
  - **Not supported**: the ungated `capture_value_refs`/`capture_value_position` passes are wired only for Python (grep: only `python.rs` calls them). So these produce nothing, even with `--reference-edges`:
    - `[1].map(mapper)` and `lodash.map([], mapper)` (fixture f10).
    - `setTimeout(cb, 1)`, `const f = viaVar; f()`, `{ run: helper }` and `o.run()` (fixture f4).
  - This contradicts README.md:1514-1517, which lists javascript among the languages whose "function-as-value capture" is covered.
- **String targets**: registrar capture also emits `STRING_REF_HINT` records for string arguments other than the route. An only argument is kept: `new Worker('./w.js')` (registrar.rs:452-462). They are used only by framework rules.
- **`fn.call/apply/bind`, `Reflect.apply`**: recorded as calls named `call`/`apply`/`bind` on receiver `fn`. They never reach `fn` (fixture f4: no edge to `helper`/`cb`).
- **Computed member** `obj["m"]()` / `this[name]()`: the `function` is a `subscript_expression`, which `record_call` drops (`_ => return`, javascript.rs:1234). Fixture f1 L60 and f4: no record.
- **Dynamic import** `import('./b.js').then(m => m.later())`: not modelled as an import. `m.later()` fell back to by-name (fixture f4 `later` medium). In fixture f2, `m.doWork()` fanned out to both `doWork`s.
- **JSX**: `<Widget />` and `onClick={this.handle}` are not calls (fixture f3: no edge to `Widget` or `Widget.handle`; also the `react-hooks` rule gap, rules.rs:11496).
- **Arity**: not recorded (`arity: None`; JS is not in `OVERLOAD_ARITY_LANGS`, type_hints.rs:1583).
- **Keyword arguments**: not applicable (`kwargs: Vec::new()`).
- **Explicit generics / paren-less calls / `path_receiver` / `dot_call`**: never set by the JS plugin.
- **Registrar verbs** for JS are the union of the JS rules' `registrars` (`registrar_verbs_for`, rules.rs:11929-11950):
  - HTTP verbs plus `all`, `use`, `route`, `on`, `once`, `addListener`, `prependListener`, `prependOnceListener`, `notFound`, `onError`.
  - `del`, `opts`, `pre`.
  - `Worker`, `Piscina`, `process`.
  - `ipcMain.*`, `ipcRenderer.*`, `contextBridge.exposeInMainWorld`, `app.on`, `app.once`.
  - `addEventListener`, `serve`, `middy`.
  - Azure: `http`, `timer`, `storageBlob`, …
  - Firebase v2: `onRequest`, `onCall`, `onDocument*`, …
  - GCP: `http`, `cloudEvent`, `typed`.
  - React Native: `registerComponent`, …
  - User-authored rules add verbs through `ExtractCtx` (test lib.rs:404: `middy`).
  - A JS/TS options object (`{ handler: async () => … }`) is searched for inline closures (registrar.rs:623).

## 04. Imports & module resolution

Import records written by the plugin:

- **ESM named import** `import { helper, scale as s } from './utils.js'` → `from-import` with alias `helper, scale as s` (record_import, javascript.rs:648-786; test `esm_imports_captured` javascript.rs:1581).
- **Default import** `import x from 'm'` → `from-import` item `default as x` (javascript.rs:734-737).
- **Namespace import** `import * as ns from 'm'` → `import` with alias `ns`.
- **Side-effect import** `import './polyfill'` → `import` with an empty alias.
- **CommonJS require**:
  - `const m = require('./m')` → `import` alias `m`.
  - `const { a, b: c } = require('./m')` → `from-import` `a, c` (try_record_require, javascript.rs:563-646). **Partial**: for `b: c` only the local name `c` is kept, not the `orig as local` pair.
  - Assigned `x = require('m')` → `import` (javascript.rs:519-561; test `export_from_and_assigned_require_are_imports` javascript.rs:1610).
- **Re-exports** (record_reexport, javascript.rs:800-838; test `es_reexports_are_recorded` javascript.rs:1426): `export * from`, `export * as ns from` and `export { a, b as c, default as D } from` become `re-export` records, plus a plain `import` record.
- **Local exports** (record_local_export, javascript.rs:870-919) become `local-export` records:
  - `export { run as go }`.
  - `export default name`, `export default function f(){}` and `export default new Api()`.
  - An unreadable value is recorded as `<value> as default` (`UNKNOWN_EXPORT`, javascript.rs:91). Test `unreadable_default_exports_are_marked_unknown` (javascript.rs:1446).
- **CommonJS exports** (try_record_cjs_export, javascript.rs:937-1072; test `local_exports_and_commonjs_exports_are_recorded` javascript.rs:1475):
  - `module.exports = ident`, `= function`, `= class X`, `= new X()`.
  - `module.exports = { a, b: c, k: require('./m'), ...require('./a') }`.
  - `module.exports = require('./m')` → `*, default` re-export.
  - `exports.k = v` and `module.exports.k = require('./m')`.
- **Package identity**:
  - `package-name` records the nearest `package.json`/`deno.json`/`jsr.json` name.
  - `package-export` records the specifiers the package's `exports` map gives this file (push_package_name, javascript.rs:96-116; manifest.rs:290-340).
  - `FileFacts::packages` holds every name and alias the file is importable under, including tsconfig/jsconfig `paths` keys.

Resolution:

- **Step 0c, module-scoped lookup**: a call written through an import of a workspace module is looked up only in the file that import names. This follows re-exports, `paths`, and `package.json`/`deno.json` `exports`; it never matches by name elsewhere (cross_file.rs:5382-5395; `ModuleIndex`, modules.rs:190-266). Tests:
  - `js_module_binding_picks_the_imported_file_in_either_order` (cross_file.rs:11827)
  - `js_require_follows_commonjs_exports` (11866)
  - `js_barrel_reexports_are_followed_and_a_missing_name_refused` (11903)
  - `js_commonjs_instance_export_reaches_its_class` (12365)
  - `js_values_and_chains_are_not_refused` (12617)
  - `js_namespace_member_called_and_nested_definitions` (12786)
  - `js_unreadable_values_are_not_proof_of_absence` (13091)
  - `js_readable_modules_stay_decisive` (13158)
  - `crates/cgg/tests/resolve.rs::js_esm_import_resolves` (1340)
- **Fixture f2**: every one of these was **high**:
  - Through a barrel and a workspace package name: `coreDefault()` (default export of `@acme/core`), `doWork()`, `core.doWork()`, and `utility()` (`export { util as utility }`) → `util`.
  - jsconfig `paths` alias `@lib/aliased` → `viaAlias`.
  - CJS `cjs.a()`, `cjs.bee()` → `b`, destructured `a()`/`bee()`.
  - Instance export `store.get()` → `Store.get` (`module.exports = new Store()`).
- **not-in-imported-module**: a name the imported module does not provide is refused. Fixture f7: `native.plain()` through `require('./index.node')` → `not-in-imported-module` (`{'module': './index.node'}`).
  - **Partial**: in fixture f2, `core.missing()` through the workspace package `@acme/core` was filed as **external** (`<external>::core::missing` with `--include-external`), not `not-in-imported-module`.
  - CHANGELOG.md:1320-1323 lists related misses: "a JS module that builds its exports at run time and an npm package the tree also has a directory named like (`next/headers`) are refused as `not-in-imported-module` rather than external".
- **External-module refusal** (`heads.rs`): every JS import binds its local name; a specifier that is not a workspace module is external (heads.rs:460-483). The workspace vocabulary is built from:
  - JS file paths.
  - `FileFacts::packages`: npm names, plus alias prefixes for `*`/`/` keys (heads.rs:107-115).
  - Tests: `npm_package_names` (heads.rs:1075), `js_path_aliases_and_workspace_packages_are_not_external` (heads.rs:1192), `a_bare_name_the_file_defines_is_never_refused` (heads.rs:1179), `js_dependency_head_refused_workspace_package_kept` (cross_file.rs:11315), `crates/cgg/tests/resolve.rs::js_dependency_call_is_external_but_a_workspace_package_resolves` (2724).
  - Fixture f2: `express()`, `_.map()` and `express().get()` were external; `readFileSync` from `fs` and `path.join` were stdlib.
- **Imports are never matched by bare name** for JS/TS: a relative import does not add the bare source name to `direct_imports` (cross_file.rs:1664-1676).
- **Global by-simple fallback (step 1d)**: a file with *any* import binds an unresolved bare name to an in-tree callable with ≤3 same-named candidates, at medium (cross_file.rs:6013-6025). Fixtures:
  - f2: a bare `start()` that nothing imported → `util.js::start` medium (one of two `start`s).
  - f7: bare `plain(3)` → `wrap.js::plain` medium, with no import of `wrap.js`.
- **Doubled reporting, cause unverified**: with `--include-external`, fixture f2 also minted `<external>::utility`, `<external>::cjs::bee` and `<external>::bee` for sites that also had a high edge (`util`, `b`). Renamed exports appear in both buckets.
- **Not supported**:
  - Dynamic `import()` (fixtures f2, f4).
  - `require(variable)`.
  - Bundler-only aliases (webpack/babel) with no tsconfig/jsconfig/deno entry (CHANGELOG.md:1214).
  - A tsconfig `paths` alias is matched by key and prefix, not resolved through its target list (CHANGELOG.md:1324-1325: "a JS/TS path alias is matched by path suffix, not read from `tsconfig` `paths` targets").

## 05. Type inference & receiver typing

- **No declared types**: JavaScript has no annotations. The plugin writes no `local_types`, `scoped_types`, `class_fields` or `module_typed_locals`. A grep of javascript.rs and typescript.rs for `local_types`, `scoped_types` and `weak_receiver_type: true` finds nothing. `collect_param_types` reads parameter types from `signature_hint`, and JS signatures carry none. README.md:487's "Type inference: params" for JavaScript is therefore not borne out (unverified, no fixture shows a parameter-typed binding).
- **Receiver shape typing** (`receiver::peel`, receiver.rs:54-135, 200): `new Foo().m()` and `(new Foo()).m()` give `typed_receiver = true` and type `Foo`. Fixture f1: `(new Child(3)).run()` → `Child.run` high.
- **Name-convention typing** (`find_constructor_assignments`, type_hints.rs:2404-2461): a receiver whose name is a lower-cased class name declared **in the same file** is typed as that class. Fixture f1: `base.step()` → `Base.step` **high** (intra-file).
  - **Partial**: the same file's `b.run()` (`const b = new Base(1)`) and `c.step()` (`const c = new Child(2)`) were **not** typed and fanned out to `Base.*` and `Child.*` (medium).
- **`const x = new Foo()` is not a typing source**:
  - Fixture f2: `const e = new Engine(); e.start()` → three `start` candidates across files (medium).
  - Fixture f5: `const d = new Dog(); d.bark()` → medium by name.
- **`this.field = new Foo()` field typing**: not supported. Fixture f9: `this.repo.find()` with `this.repo = new Repo()` produced **no edge** (`candidates-in-other-files`, 1 candidate) although `Repo.find` is in the same file.
- **`declared_type` tables**: `NULL_TYPE_MEMBERS` has a `javascript` row (`null`, `undefined`, `void`; stdlib.rs:134). It has no effect without annotations; the row exists because the walker is shared with TypeScript. JS has no `MAYBE_TYPE_WRAPPERS` row (TypeScript does, stdlib.rs:120).
- **Return-type propagation**: `build_return_type_map` reads declared return types, which JS has none of. `javascript` appears in `lang_tallies_table!` (type_hints.rs:336-353), but only for profiling counters.
- **Closure parameter typing, implicit receivers, casts** (`CAST_INTRINSICS`), `CONSTRUCTING_METHODS`: none for JS (stdlib.rs:35-58).
- **Weak receiver types**: never set.
- **Call operator** (step 7, `call_operator_names`): no JS entry (cross_file.rs:394-400).

## 06. Inheritance, dispatch & overloading

- **Base records**: `extends X` on a class is copied into each method's `base_types` (section 02). This is read by `resolve_via_bases` and by `dispatch::inheritance_fanout`. JS is in `INHERITANCE_LANGS` (dispatch.rs:110-123) and not in `CLASS_RECORDS_ARE_EXACT` (dispatch.rs:131).
- **`super.m()`**:
  - Fixture f1 (same file): `super.step()` → `Base.step` **high**, `super.run()` → `Base.run` high.
  - Fixture f5 (base imported from another file): `super.speak()` → `Animal.speak` **medium**.
  - JS is not one of the "every class recorded" languages, so `super-base-out-of-graph` refusal does not apply (CLAUDE.md lists Python, Kotlin, Java, C#).
- **Inherited `this.m()`**: fixture f5 `this.speak()` in `Dog` → `Animal.speak` medium.
- **Inherited static**: fixture f5 `Dog.create()` → `Animal.create` medium.
- **`this.m()` from a class field initializer**: ambiguous. Fixture f1 L18: `this.run()` inside `field = () => this.run()` → `ambiguous-in-file` (`Base.run`, `Child.run`).
- **`member_of_calling_class` / `overrides_below`**: Java/C#/Ruby/Scala features. For JS, a bare call never reaches a method (`bare_call_is_never_a_method`, intra_file.rs:49-54; cross_file.rs:4137-4139), so they do not apply.
- **`--dynamic-dispatch` / `inheritance_fanout`**: fixture f5 with `--dynamic-dispatch` added `Animal.sound -> Dog.sound` (low, `dispatch:inheritance`, via `dynamic`). It also runs under `--dead-code`.
- **Interface/trait impls**: no JS construct. `implements` is TypeScript.
- **Prototype inheritance**: not modelled.
  - `Legacy.prototype.greet = function(){}` is an owner-less `greet`, so `l.greet()` binds by name only (fixture f4 medium).
  - `Object.assign(X.prototype, {…})` mixins: not modelled (fixture f4 `wave`).
  - `extends mixin(Base)` (a call expression in the heritage clause): **unverified**. `attrs::base_types` keeps only type-name-shaped parts (attrs.rs:199-213).
- **Extension methods, traits, included modules**: not applicable.
- **Arity overload narrowing**: not applicable (JS is not in `OVERLOAD_ARITY_LANGS`, type_hints.rs:1583).
- **Duck-typed fan-out**:
  - An untyped receiver fans out to every same-named callable up to `--fanout-cap` (default 5), **including free functions**. Fixture f5: `x.sound()` → `Animal.sound`, `Dog.sound` and the free function `sound` (all medium).
  - Over the cap, nothing is bound. Fixture f5: `x.close()` with 7 `close`s → `candidates-in-other-files {'candidates': 7}`.

## 07. Scoping & shadowing

- **`FileFacts::locals`**: the JS spec (`JS_SPEC`, locals.rs:152-181) covers parameters (including destructured and rest), `variable_declarator` names, `for…in/of` bindings, `catch` parameters and arrow parameters. Bindings are collected only **inside functions**: module-level `const top` is not a binding. Test `javascript_params_locals_and_closure_params` (locals.rs:696). A declaration is not in scope in its own initializer (test `a_declaration_is_not_in_scope_in_its_own_initializer`, locals.rs:714, JS case at 742).
- **shadowed-by-local**: fixture f5 `function shadowTest(a1) { return a1(); }` → `a1` recorded `shadowed-by-local`, never bound to the file's `a1`. Test `a_parameter_or_local_shadows_the_files_function` (intra_file.rs:1102). A local equal to its own `require`, or a same-named member alias (`var pick = H.pick`), keeps the import reading (CHANGELOG.md:561-577; test `a_same_named_member_alias_is_not_a_binding`, locals.rs:810).
- **Lexical narrowing of nested definitions**: JS is in `lexically_scoped` (intra_file.rs:118-135). It applies only when the file's locals were read, so not in minified files (intra_file.rs:360-366). Fixture f5: `outerA -> inner` and `outerB -> inner` each bound to their own nested `inner` (high).
- **Nested named functions** are extracted with the enclosing *class* scope, not the function scope. Fixture f1: `nested`, `nestedArrow` inside `outer` have QNs `nested`, `nestedArrow`.
- **A local named arrow shadows a same-named global**: fixture f5 `const close = () => 1; close()` → the local `close` (high).
- **Positional shadowing** (OCaml/F#/Lua): not applied to JS.
- **Conditional definitions** (`DefRecord::conditional`, PHP): not applicable. Two same-named definitions in one file stay ambiguous: test `a_repeated_name_in_a_language_without_overloading_stays_ambiguous` (intra_file.rs:1423) uses a JS minified-bundle case.
- **Module-level bindings**: a module-level `const f = viaVar` is neither a local nor a definition, so `f()` produced nothing (fixture f4).

## 08. Macros, preprocessor & codegen

- **Macros / `#define` / `#if`**: not applicable. JavaScript has no macro system.
- **Generated code tagging (`macro:<name>`)**: not applicable.
- **`name_mentions`**: not written by the JS plugin (grep of javascript.rs finds no `name_mentions`).
- **JSX**: parsed, but not transformed into calls (section 03).
- **Bundler/transpiler output** (Babel, webpack): analyzed as ordinary JS unless `--skip-minified` (section 01).
- **Synthetic closures** (`handler_at_<line>`, attribute `synthetic`) are the only generated nodes (section 02).

## 09. FFI & cross-language links

- **napi / wasm-bindgen**: `allowed_source_languages("napi")` = `javascript`, `typescript` (ffi.rs:694-701). Pass B (ffi.rs:188-260) links an exported Rust callable to a **same-simple-name callable** in JS (callable → callable, medium, `Via::Ffi`). It does not link from a JS *call site*.
  - Fixture f7: a JS `function plain()` → Rust `native::plain` (medium, `ffi-linker`).
  - The Rust `#[napi] fn sum_values` was **not** linked to a JS `sumValues`: there is no snake_case → camelCase mapping (grep for `camel` in ffi.rs finds nothing).
  - Test `wasm_bindgen_links_rust_to_javascript` (ffi.rs:910).
- **JS → native module calls**: `require('./index.node')` names a file cgg does not analyze. Calls through it are `not-in-imported-module` (fixture f7).
- **PyO3, JNI, P/Invoke, extern C, asm**: not applicable to JS.
- **Descriptor links** (`descriptor::link_descriptors`, descriptor.rs:80-145): language-agnostic. A protobuf `rpc` binds to a host method whose owner contains the service name (case-insensitive). Fixture f8: `Greeter.SayHello` (proto) → `GreeterService.sayHello` (JS), low, `descriptor-linker`.
- **Markup code-behind** (`markup.rs`, `markup_kinds_for`): C# only (rules.rs:11812-11819). Not applicable.

## 10. Frameworks & entry points

Every `RuleSpec` with `language: "javascript"` (37 rules, extracted from `crates/cgg-core/src/frameworks/rules.rs`). Unless noted, `node: true`. "detect" is the import specifier prefix that turns a rule on.

| rule id (line) | kind | trigger | enumerates? |
| --- | --- | --- | --- |
| `express` (407) | network | detect `express`; registrars get/post/put/delete/patch/head/options/all/use/route | yes. Fixture f3: `express::get("/users") -> listUsers` and `post("/admin") -> handler_at_8`. Tests `shape_b_express_named_handler_binds_to_its_route`, `shape_c_anonymous_handler_still_gets_its_route_named` (tests/frameworks.rs:131,165). `app.use(arrow)` with no route minted only a synthetic closure, with no entry (fixture f3 `handler_at_9`) |
| `bullmq` (494) | queue | detect `bullmq`, `bull`; registrars Worker/process/on | yes (unverified by fixture) |
| `worker-threads` (528) | queue | detect `worker_threads`, `piscina`, `node:worker_threads`; registrars Worker/Piscina; `self_module_markers_for` = `parentPort`, `workerData` (rules.rs:11666-11673) | module-path entry for an uncalled, non-synthetic function of the worker file (frameworks/mod.rs:1780-1870; test `shape_f_worker_module_path_rescues_a_whole_file`, tests/frameworks.rs:360). Fixture f3 got none: the worker's only function was called from a synthetic closure ("detected, but no entry point matched") |
| `nextjs` (565) | network | detect `next`, `next/` | **no**: gap "routes come from file-system layout". Test `coverage_reports_a_detected_framework_it_cannot_enumerate` (frameworks/mod.rs:2503) |
| `koa` (1717), `hapi` (1751), `fastify` (1785), `remix` (1819), `sveltekit` (1853), `nuxt` (1887), `astro` (1921) | network | detect only | **no**: each has a `gap` text |
| `electron` (1967) | queue | detect `electron`; registrars `ipcMain.handle/handleOnce/on/once`, `ipcRenderer.on/once`, `contextBridge.exposeInMainWorld`, `app.on/once` | yes. Test `electron_ipc_handlers_are_entries_and_other_on_calls_are_not` (tests/frameworks.rs:199) |
| `socket-io` (2027) | network | detect only | **no** (gap) |
| `cloudflare-workers` (2061) | network | detect `cloudflare:*`, `@cloudflare/*`; registrar `addEventListener`; methods fetch/scheduled/queue/email/trace/tail | yes. Test `cloudflare_legacy_addeventlistener_binds` (tests/frameworks.rs:1136) |
| `deno-http` (2141) | network | detect `jsr:@std/http`, `@std/http`, `https://deno.land`; registrar `serve`; method `fetch` | yes (unverified by fixture) |
| `hono` (2192) | network | detect `hono`, `@hono`; HTTP verbs plus all/use/on/notFound/onError | yes (unverified by fixture) |
| `aws-lambda` (4129) | network | detect `aws-lambda`, `@aws-lambda-powertools`, `@middy/core`; registrar `middy`; methods `handler`, `lambdaHandler` | yes. Fixture f3: `aws-lambda::handler -> handler` (exports.handler) and `aws-lambda::base -> base` (middy-wrapped). The gap notes detection is per language, run-wide |
| `azure-functions` (4412) | network | detect `@azure/functions`; registrars http/timer/storageBlob/… | yes, the v4 options-object `handler`. Test `azure_functions_binds_the_attribute_and_the_options_object` (tests/frameworks.rs:1040). The v3 model is invisible (gap) |
| `firebase-functions` (4473) | network | detect `firebase-functions`; v2 registrars onRequest/onCall/onDocument*/… | yes, v2 only |
| `gcp-functions` (4792) | network | detect `@google-cloud/functions-framework`; registrars http/cloudEvent/typed | yes. Test `gcp_functions_binds_every_runtime_shape` (tests/frameworks.rs:1001). Legacy `exports.x` invisible (gap) |
| `node-event-emitter` (6918) | lifecycle | detect `events`, `node:events`, `node:process`, `node:stream`, `stream`, `eventemitter3/2`; registrars on/once/addListener/prependListener/prependOnceListener | yes. Fixture f3: `on("data") -> onData`, `on("exit") -> handler_at_7`, `on("message") -> handler_at_3` (the last from `parentPort.on` in the worker) |
| `feathers` (8442), `graphql-yoga` (8497), `apollo-server` (8543), `express-gateway` (8568), `meteor` (8585), `sails` (8602) | network | detect only | **no** (gaps: resolvers and services are object properties or config strings) |
| `restify` (8619) | network | registrars get/post/put/patch/del/head/opts/pre | yes (unverified by fixture) |
| `polka` (8663) | network | HTTP verbs plus all/use | yes (unverified by fixture) |
| `vitest` (8716), `jest` (8750), `playwright` (8820) | **test** | detect only | **no** (gap; see section 11) |
| `react` (11447) | lifecycle, `node: false` | detect `react`, `react-dom`; methods componentDidMount, componentDidUpdate, componentWillUnmount, componentDidCatch, shouldComponentUpdate, getSnapshotBeforeUpdate, getDerivedStateFromProps, getDerivedStateFromError, componentWillMount/ReceiveProps/Update and `UNSAFE_` variants | root mark only. Fixture f3: `Widget.componentDidMount` root (`react:base-type`). **`render` is not in the list**: fixture f3 reported `Widget.render` dead (medium) |
| `react-hooks` (11496) | lifecycle | detect `react` | **no** (gap: JSX element names, hook callbacks and `on*` props are not calls) |
| `react-native` (11513) | lifecycle | registrars registerComponent/registerHeadlessTask/registerRunnable/registerRootComponent | yes (unverified by fixture) |
| `vue` (11574), `svelte` (11608) | lifecycle | detect only | **no** (`.vue`/`.svelte` not parsed) |

- **Shapes supported** (registrar.rs header):
  - B: a named handler argument.
  - C: an inline closure (synthetic `handler_at_<line>`).
  - E: a string target, through `string_targets`. No JS rule sets `string_targets: true`.
  - F: a worker module path.
  - Test `ordinary_callbacks_do_not_mint_synthesized_handlers` (tests/frameworks.rs:229) and `a_registrar_binds_the_handler_named_in_argument_position` (frameworks/mod.rs:2589).
  - Test `a_registrar_naming_nothing_visible_is_dropped_not_guessed` (frameworks/mod.rs:2756).
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for`**: no JS entries (rules.rs:11686-11846).
- **Detection prefixes** are exercised by `crates/cgg/tests/detect_prefixes.rs` (JS form `const _x = require('{prefix}')`, line 36).
- **Decorator (attribute) rules**: no JS rule uses `attributes`. Decorator matching is TypeScript/NestJS.

## 11. Test harness entries

- **Rules of kind `test`** for JS: `vitest`, `jest` and `playwright` (rules.rs:8716, 8750, 8820). All three are detect-only with a `gap`, so **no `<framework-entry>::test::…` harness node is ever minted for JS**. There are no `mocha`, `node:test`, `ava`, `tap`, `jasmine` or `cypress` rules (grep finds none). Fixture f3 (`test/server.test.js` with `describe`/`it`/`test`, `jest` in `devDependencies`): no test entry.
- **Test blocks are not callables**: `describe('…', () => …)` and `it(…)` arrows are not extracted. Calls inside them have no enclosing callable (fixture f3: `fixtureHelper` → `no-enclosing-callable`), which makes the callee a `toplevel:invocation` root in the dead-code report.
  - A **named** test function (`test('top', function namedTest(){…})`) is extracted. It was reported `never-referenced` (fixture f3 dead-code JSON).
- **Test-file classification** (`testfile.rs`):
  - By name, `.test.{js,jsx,mjs,cjs}` and `.spec.{js,jsx}` (testfile.rs:84-97). **Gap**: `.spec.mjs`/`.spec.cjs` are not listed.
  - By directory, `test`, `tests`, `__tests__`, `spec`, `specs`, `testdata`, `e2e` (testfile.rs:35-45; test `generic_test_directories_are_the_weakest_signal`, testfile.rs:208).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: python, php and ruby only (rules.rs:11806). Not JS.
- **`test_defs` signal**: not declared, so the dead-code capability table shows `test_tagging: none` (fixture f3 dc.json).

## 12. Dead-code analysis

- **Capability row** (fixture f3 `dc.json`): `class: degraded`, `visibility: none`, `attributes: full`, `value_references: full`, `dispatch: full`, `exports: none`, `test_tagging: none`, **`max_confidence: medium`**.
  - The class is `Degraded` because the plugin does not declare `visibility` (caps.rs:163-173).
  - JS findings therefore never reach `high`: every finding in fixtures f3/f9 is `medium` or `low`.
- **Visibility**: class members get `Vis::Public`/`Private` (`#name`) through `class_member_vis` (javascript.rs:1364-1390). Module-level `export` is not modelled, so the signal is not claimed (javascript.rs:338-342, README.md:1527-1530). Evidence kinds seen: `language-lacks-visibility`, `public-visibility`.
- **Exports are not roots**: the JS plugin writes no `FileFacts::exports`, so the "declared exports are public API" root (roots.rs:890-915) never fires for JS. Fixture f3: `export function FnComp` reported `never-referenced`.
- **Roots seen** (fixture f3 `roots`):
  - `framework:entry-node` (synthesized entries).
  - `framework:rule` (`react:base-type` for `componentDidMount`).
  - `toplevel:invocation`, a call from module top level (`em.on('data', onData)`, `app.get('/users', listUsers)`, and the call inside an anonymous `it(…)` block).
- **Conventional names**: JS is in `DUNDER_LANGS` (roots.rs:160-161), and `CONVENTIONAL_METHODS` (`toString`, `describe`, …; roots.rs:162-170) are roots.
- **`main`**: `UNIVERSAL_ENTRY` (roots.rs:20).
- **Implicitly invokable kinds**: getters, setters and constructors carry `implicitly-invokable-kind` evidence (fixture f9: `v`, `Svc.constructor`).
- **Doubt**: a same-named unresolved site downgrades a finding to `low` with `name-matches-unresolved-site` (fixture f9: `Repo.find` low).
- **Blind spot** listed for JS: "obj[expr] dynamic property access" (caps.rs:57).
- **Unreachable code**: `cfg::unreachable_after_terminator` with the `JS` spec runs under `--dead-code`. The spec has return/throw/break/continue terminators and hoisted function/class/var declarations (cfg.rs:135-150; javascript.rs:74-76; tests at cfg.rs:297-303).
  - **No consumer of `FileFacts::unreachable` was found** in `cgg-resolve` or `cgg` (grep for `.unreachable` hits only the plugins writing it).
  - Fixture f9: `afterReturn()` after a `return` still produced `dead -> afterReturn` (high).
- **Caps**: `implements-external-contract` (needs `@Override`/`override` or an outside base). A JS class extending an outside base (`React.Component`) is not shown capped (fixture f3: `Widget.render` medium, but that is already the language cap). `macro-generated` is not applicable. `reflection` caps come from `dyn_uses` (section 15).
- **Synthetic closures and harness**: `handler_at_*` closures are rooted through their entry nodes.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/javascript.txt`**: 569 names, loaded as `("javascript", parse(JAVASCRIPT))` (stdlib.rs:187, 228).
  - Includes globals and methods: `console`, `log`, `map`, `then`, `push`, `join`, `AbortController`, …
  - Includes regex `$1`…`$9`.
  - Includes Node core module names (`fs` at line 176, `path` at 332).
- **Use**:
  - The step 1d by-simple fallback skips stdlib names (cross_file.rs:6022-6024).
  - Stdlib heads are classified in heads.rs:794.
  - With `--include-stdlib`, calls become `<stdlib>::…` exit nodes. Fixture f2: `readFileSync` (imported from `fs`) → `<stdlib>::readFileSync`, `path.join` → `<stdlib>::path::join`.
  - With `--include-external`, external package calls become `<external>::…` (fixture f2: `express`, `_::map`).
- **`CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `TOP_TYPES`, `CALLBACK_TYPE_CONSTRUCTORS`, `UNION_TYPE_CONSTRUCTORS`, `MAYBE_TYPE_WRAPPERS`**: no JS row (stdlib.rs:35-160).
- **`NULL_TYPE_MEMBERS`** has a `javascript` row (stdlib.rs:134). It is inert without annotations.
- **Implicit packages** (`SCALA_IMPLICIT_PACKAGES`-style): none for JS. JS globals are covered by the stdlib name list instead.
- **`rust_types`-like wrapper tables**: not applicable.

## 14. Confidence & certainty

- **High** (observed):
  - Intra-file bare calls (`plain`, `helper`).
  - `this.m()` within the declaring class (`Base.run -> Base.step`).
  - `this.#m()`.
  - Optional-chained and tagged-template calls.
  - A typed receiver from `new X()` in the expression (`(new Child(3)).run()`).
  - The same-file name-convention receiver (`base.step()`).
  - Same-file `super.m()`.
  - Import-anchored cross-file calls (fixture f2: ESM, CJS, barrel, workspace package, jsconfig alias, instance export; `CertainCtx::path`, cross_file.rs:4485-4497).
  - `new Dog()` imported → `Dog.constructor` high (fixture f5).
- **Typed-owner anchoring**: for JS the candidates must be **in the calling file** (`anchored`, cross_file.rs:4583-4605: `"javascript" | "typescript" => in_caller_file`). A bare-type-name lookup across files therefore stays medium.
- **Medium**:
  - Duck-typed and by-name fan-out (`b.run()`, `x.sound()`, `getThing().run()`).
  - Cross-file inherited members and cross-file `super` (fixture f5).
  - Step 1d by-simple fallback (fixtures f2, f7).
  - Constructor mapping through step 6 when the class is not directly imported (fixture f1 `Base.constructor` medium).
  - Object-literal method by name (`obj.m()` → `m`).
  - FFI Pass B.
  - `--reference-edges` edges.
- **Low**: framework-entry edges, descriptor links, inheritance dispatch (`--dynamic-dispatch`).
- **Never cross-file high**: a minified calling file (`minified_caller`, cross_file.rs:4545-4550). Intra-file edges in a minified file remain high (fixture f11).
- **Mermaid `?` marker**: language-agnostic. An arrow with no `high` site gets a leading `?` (CLAUDE.md, cgg-format mermaid).

## 15. Dynamic use & reflection

- **`dynuse` coverage**: `REFLECTIVE` for `javascript` = `require`, `importScripts` (dynuse.rs:49). Both are in `DYN_MODULE_LOADERS` (facts.rs:165), so they count as **module-load** screens, which do not cap findings. JS has **no reflection screen**: `obj['name']()`, `this[name]()`, `Reflect.get/apply`, `globalThis['f']` and `eval` are not captured. Extraction runs only under dead-code signals (javascript.rs:77-79). The plugin does not declare the `dyn_uses` signal (javascript.rs:35-43).
- **Computed member calls** are dropped at extraction (section 03). This is the blind spot "obj[expr] dynamic property access" (caps.rs:57).
- **`eval`, `new Function(...)`**: recorded as ordinary calls named `eval`/`Function`. They are opaque; fixture f4 showed no edge to `helper` from `eval('helper(1)')`.
- **`require(variable)`, dynamic `import(x)`**: no import record and no dyn-use (`require(name)` with a non-literal argument is ignored, dynuse.rs:117-125).
- **`Function.prototype.call/apply/bind`**: not followed (fixture f4).

## 16. Known gaps & limitations

- **Callables missing**: class fields holding functions (`handle = () => …`, very common in React class components); anonymous object-literal function values; anonymous default exports; anonymous IIFEs; `static {}` blocks; test `describe`/`it` blocks (fixtures f1, f4, f3, f9).
- **Owner-less QNs**: object-literal methods, `X.prototype.m = …`, `exports.m = …` and nested functions get bare QNs. Prototype classes cannot be told apart, and calls on them bind by name (fixtures f1, f4).
- **Duplicate QNs** for getter/setter pairs (fixture f1 `Base.label` ×2).
- **No local type inference**: `const x = new Foo(); x.m()` and `this.f = new Foo(); this.f.m()` are not typed (fixtures f2, f5, f9). This is the main source of medium fan-out.
- **Value references are registrar-gated only**: `.map(fn)`, `setTimeout(fn)`, `{ k: fn }` and `const f = g` produce no reference record, even with `--reference-edges` (fixtures f4, f10). This contradicts README.md:1514-1517.
- **Dynamic import / computed access / call-apply-bind** are not followed (fixture f4).
- **JSX element names and `on*` props are not calls**. Function components and JSX-bound handlers read as dead (`react-hooks` gap; fixture f3).
- **The `react` rule omits `render`** (rules.rs:11447-11470; fixture f3 `Widget.render` dead).
- **No enumerating test harness**: jest/vitest/playwright are gap-only (section 11).
- **Many detect-only web frameworks**: koa, hapi, fastify, remix, sveltekit, nuxt, astro, socket-io, feathers, graphql-yoga, apollo-server, express-gateway, meteor, sails, nextjs (`gap_for`, rules.rs:11870-11876).
- **Named-function-in-registrar duplicate**: `on('x', function f(){})` mints both `f` and `handler_at_N` over one span; the entry targets the synthetic one and `f` reads dead (fixture f3).
- **Express `app.use(fn)` without a route**: no entry node (fixture f3 `handler_at_9`).
- **Dead-code ceiling is medium** (no `visibility`/`exports` signal); exports are not roots (section 12).
- **Unreachable regions are computed but unused**: no consumer was found by grep (section 12).
- **No camelCase napi mapping** (section 09).
- **External vs not-in-imported-module** misfiling for an unknown member of a workspace package (fixture f2 `core.missing`). CHANGELOG.md:1320-1325 records related misses: run-time-built exports, `next/headers`-style directories, and tsconfig `paths` targets.
- **Bundler-only aliases** are not read (CHANGELOG.md:1214).
- **WS14 follow-up**: "a JS/TS prop holding a bound action creator of the same name when the file does not import it" (CHANGELOG.md:1351-1352).
- **Shebang**: only the `node` prefix. `bun`/`deno` scripts are classed as TypeScript (fixture f6).
- **`.es6`, `.vue`, `.svelte`, `.astro`** are not analyzed.
- **Parse errors** can misclassify functions as methods (fixture f6 `ok`).
- **Ignored tests**: none (`grep -rn '#\[ignore'` over `crates/` returns 0).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | full | `.js/.mjs/.cjs/.jsx` and the `node` shebang; JSX via the JS grammar; minified handling; no error-recovery re-parse |
| 02 Callables extracted | partial | functions, arrows, classes, accessors, CJS assignments; class-field functions, anonymous object values and IIFEs missing; owner-less prototype and object members |
| 03 Call-site forms | partial | bare, member, `new`, optional, tagged, chained; computed access, call/apply/bind, JSX, dynamic import missing; value refs only on registrar verbs |
| 04 Imports & module resolution | full | ESM, CJS, re-export barrels, default/instance exports, package.json/deno/jsconfig/tsconfig names and exports; dynamic import missing |
| 05 Type inference & receiver typing | minimal | only `new X().m()` shape and the same-file lowercase-name convention; no `const x = new X()` or field typing |
| 06 Inheritance, dispatch & overloading | partial | `extends` bases, `super`, inherited members, inheritance fan-out; prototype/mixin inheritance not modelled; no overloads |
| 07 Scoping & shadowing | full | locals, shadowed-by-local, lexical nesting (not in minified files) |
| 08 Macros, preprocessor & codegen | n/a | no macro system; only synthetic `handler_at_*` closures |
| 09 FFI & cross-language links | minimal | napi/wasm-bindgen callable-to-callable by exact name only; descriptor links work |
| 10 Frameworks & entry points | partial | 37 rules; about 16 enumerate (express, hono, electron, lambda, cloud functions, event emitter, …); about 21 are detect-only gaps |
| 11 Test harness entries | none | jest/vitest/playwright are gap-only; test blocks are not callables |
| 12 Dead-code analysis | partial | capped at medium (degraded: no visibility/exports signal); exports not roots; unreachable regions unused |
| 13 Builtins & stdlib tables | full | 569-name stdlib list including Node core modules; no cast/construct tables (none needed) |
| 14 Confidence & certainty | partial | high for intra-file, `this`, and import-anchored calls; typed-owner high only within the caller's file; much medium fan-out |
| 15 Dynamic use & reflection | minimal | only `require`/`importScripts` module-load screens; no reflection screen for `obj[k]` |
| 16 Known gaps & limitations | n/a | see list above |
