# typescript

The TypeScript plugin (`TypeScriptPlugin`, `crates/cgg-lang/src/plugins/typescript.rs`, 332 lines including tests) is a thin wrapper. It owns detection, grammar choice and the `signals()` manifest, and hands extraction to the JavaScript plugin's `JsWalker` (`plugins/javascript.rs`, 1,621 lines; typescript.rs:85-86). The walker ignores type annotations, so TypeScript extracts exactly what JavaScript extracts. A few node kinds exist only in TypeScript and the walker handles them: `interface_declaration`, `abstract_class_declaration`, `method_signature`/`abstract_method_signature`, `import_require_clause`, `export =` and `accessibility_modifier`.

Grammar and resolver:

- **Grammar.** The `tree-sitter-typescript` crate from crates.io, `= "0.23"` (Cargo.toml:85). Nothing is vendored. The plugin has two grammars: `LANGUAGE_TYPESCRIPT` for `.ts`/`.mts`/`.cts` and `LANGUAGE_TSX` for everything else (typescript.rs:44-62).
- **Resolver.** There is no TypeScript-specific resolver kind. Resolution goes through the generic intra-file and cross-file passes. Many arms are written `"javascript" | "typescript"`: intra_file.rs:52, 124, 365; cross_file.rs:386, 1670, 4137, 4605, 5388, 5608; heads.rs:107, 460, 664; modules.rs:190, 266.

Declared signals (typescript.rs:33-42): `unreachable`, `dyn_uses`, `attributes`, `impls` and `value_refs`. The plugin does **not** declare `visibility`, `exports` or `test_defs`. The dead-code capability table at runtime matches this: `visibility: none, exports: none, test_tagging: none, max_confidence: medium` (fixture f5).

Fixtures: every `fN` cited below lives under `<scratch>/parity/typescript/fN/`. Each was run with `target/release/cgg . -t json -o out.json` (plus the flags named) and read from `out.json` and `out.json.audit.json`.

## 01. Detection & parsing

**Supported**

- **Extensions `.ts`, `.tsx`, `.mts`, `.cts`** (typescript.rs:22-24).
  - Fixture f6: `a.mts` is detected via `extension:.mts` and `b.cts` via `extension:.cts`.
  - detect.rs:683-684 test table has `("e.ts","typescript")` and `("f.tsx","typescript")`.
- **Shebangs `ts-node`, `tsx`, `deno`, `bun`** (typescript.rs:25-27).
  - Fixture f6: `#!/usr/bin/env -S deno run` is detected as `shebang:deno`, and `#!/usr/bin/env bun` as `shebang:bun`.
- **Per-file grammar** via `ts_language_for` (typescript.rs:54-62). `.ts`/`.mts`/`.cts` use the TypeScript grammar, so `<Foo>x` is a type assertion and `<T>(x) => x` is a generic arrow. `.tsx` and any other extension use TSX.
  - Test: `ts_files_use_the_typescript_grammar` (typescript.rs:117).
  - Fixture f6: `gen.ts` with `const r = <T,>(x: T) => x` and `<Foo>bar` parses with status `ok`, and `caller2 -> g` and `caller2 -> r` resolve at `high`.
- **Parse errors are tolerated.** The file is still extracted with `parse_status = "error"`. This is generic behaviour (cgg/src/lib.rs ~457).
- **Depth skip.** A tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `TooDeep` (cgg/src/lib.rs:155, 451). This is generic.
- **Minified handling.**
  - `locals::collect` is skipped when any line is over 2,000 bytes (`MINIFIED_LINE_BYTES`, cgg/src/lib.rs:159).
  - JS/TS lexical narrowing is applied only when locals were read (intra_file.rs:363-366).
  - `--skip-minified` (walker) drops `*.min.js`-style files. That walker test is JS-named (cgg-walk/src/lib.rs:564-643).
- **Built-in deny of `node_modules`** (cgg-walk/src/lib.rs:50).
- **`tsconfig.json`/`jsconfig.json`, `package.json`, `deno.json(c)` and `jsr.json`** are read per file for package names and alias keys (`manifest::js_module_manifest`, manifest.rs:229-286). JSON-with-comments is accepted (`read_jsonc`, manifest.rs:438; test `jsonc_comments_and_trailing_commas`).

**Not supported / partial**

- **The `ts-node` shebang is classified as JavaScript.** Detection takes the first plugin whose shebang keyword is a *substring* of the interpreter word (detect.rs:64-73). `javascript` is registered before `typescript` (plugins.rs:127-128), and `"ts-node".contains("node")`. Fixture f6: `#!/usr/bin/env ts-node` gives `javascript shebang:node`.
- **No content detection.** Nothing sniffs TypeScript inside `.js` or extensionless files.
- **No re-parse fallback.** There is nothing like C#'s `#if` blanking. A `.tsx` file that uses `<T>x` assertions or `<T>(x)=>` generics (illegal in TSX) is parsed as an error.
- **`.d.ts` files are walked**, but their `declare` items produce no callables (see §02).
- **`.vue`, `.svelte` and `.astro` single-file components are not parsed.** The rule gaps say so (rules.rs:11556 `vue`, 11590 `svelte`, 1943 `astro`).

## 02. Callables extracted

All shapes below were confirmed by fixture f1 (`shapes.ts`) unless another fixture is named. Qualified names are formed by `JsWalker::qn`, which joins the class/interface scope stack with `.` (javascript.rs:150-156). Qualified names carry **no module or file path**: a top-level `plain` is just `plain`, and `Box.run` is just `Box.run`.

**Supported**

- `function plain(a: number) {}` → `FreeFunction` `plain` (`record_function`, javascript.rs:283).
- `function* gen(){}` and `async function f(){}` → `FreeFunction` (`generator_function_declaration`, javascript.rs:162).
- `const arrow = (x: string) => x`, `const fexpr = function named(){}`, `let g = function*(){}` → a `FreeFunction` named after the variable (`try_record_named_fn`, javascript.rs:472). Only the `arrow_function`, `function_expression` and `generator_function` value kinds count.
  - Fixture f1: `fexpr` is recorded and `named` is not.
- **Overloaded functions.** `function over(a: string): void; function over(a: number): void; function over(a: any) {...}` gives one callable, `over`, for the implementation body only. The overload signatures (`function_signature`) are not definitions (fixture f1: one `over`, line 9).
- **Class members** (`record_method`, javascript.rs:308-357):
  - `constructor` → `Constructor`.
  - `get x()` / `set x()` → `Property`. Fixture f1 records two `Box.size` entries, one for the getter and one for the setter.
  - `static make()` → `StaticMethod`.
  - Others → `InherentMethod`. This covers `async *iter()`, `#secret()` (qn `Box.#secret`), computed `[Symbol.iterator]()` (qn `Box.[Symbol.iterator]`) and string-named `"quoted name"()` (qn `Box."quoted name"`).
  - Test: `class_with_types` (typescript.rs:212).
- **Class expressions.** `const C2 = class Inner { m2(){} }` gives `Inner.m2`. The scope is the class's own name, not the variable (fixture f1).
- **`abstract class`** (`abstract_class_declaration`, javascript.rs:168) owns its members. `abstract draw(): void;` → `TraitMethod` with attribute `abstract` (`record_signature`, javascript.rs:359).
- **Interface methods.** `interface Shape { area(): number; }` → `Shape.area`, `TraitMethod`, attributes `["abstract"]`, `vis = Public` (javascript.rs:193-221). Interface properties (`size: number`, `cb: () => void`) are not callables.
  - Test: `interface_and_abstract_members_are_bodiless_declarations` (typescript.rs:297).
- **Object-literal members.**
  - `{ m() {} }` → `m` (InherentMethod) and `{ get g() {} }` → `g` (Property). Neither has an owner: the qn is `m`, not `obj.m`, because `method_definition` is recorded with the current class scope, which is empty here (fixture f1).
  - `{ p: function pn() {} }` → `pn` (`extract_named_fn_child` on `pair`, javascript.rs:260, 1115).
- **Named function expressions.**
  - Returned: `return function retNamed() {}` → `retNamed` (javascript.rs:260).
  - Passed as an argument: `defineGetter(o, 'x', function name(){})` → `name` (`extract_named_fn_args`, javascript.rs:1151).
- **CommonJS assignment.** `exports.cjs = function cjsFn(){}` → `cjs`, and `module.exports.other = () => 1` → `other`. Chained `a.b = c.d = function(){}` also works (`extract_assignment_fn`, javascript.rs:406). Fixture f1.
- **Nested functions.** `function outer(){ function inner(){} const innerArrow = () => inner(); }` → `inner` and `innerArrow`. Each is its own node, and the qn is **not** prefixed by `outer`.
- **Local arrows inside a method** get the *class* scope. In fixture f8, `const helper = () => 1` inside `Holder.act` became the callable `Holder.helper`.
- **Synthesized inline handlers.** `app.post('/users', (req,res) => {...})` → `NamedClosure` `handler_at_<line>` with attribute `synthetic`. This happens only when the callee's last segment is a registrar verb of some TypeScript rule and the call has the registration shape (`extract_inline_handler`, javascript.rs:1268-1352). Fixture f5: `handler_at_6`.
- **Decorators** on methods and functions are collected as attributes (`attrs::collect_with_preceding`, javascript.rs:342). Fixture f1: `Decorated.onClick` has `["@HostListener('click')"]`.
- **Base types** from `extends`/`implements` (`attrs::base_types`, attrs.rs:187) are copied onto each member's `DefRecord::base_types`.
  - Test: `A.draw` has `base_types == ["B","Shape","L<X>"]` (typescript.rs:313).
- **Class-member visibility** (`class_member_vis`, javascript.rs:1364):
  - `private`/`protected`/`public` → `Vis::Private`/`Protected`/`Public`.
  - `#name` → `Private`.
  - No modifier → `Public`.
  - Free functions and object-literal members → `Unknown`.
  - `DefRecord::visibility` (the native token) is deliberately left empty.
  - Tests: `class_member_visibility_is_read_from_the_grammar` (typescript.rs:224) and `functions_and_object_methods_have_no_visibility` (typescript.rs:276).

**Not supported / partial** (all from fixture f1 unless noted)

- **Class fields holding functions.** Neither `handler = () => { … }` (the common React/Angular handler pattern) nor `static sfield = function sf(){}` is a callable. Calls inside them have no enclosing callable: f1's `this.helper()` at line 17 is `no-enclosing-callable`.
- **Object-literal arrow properties** (`{ q: () => 1 }`) are not extracted.
- **`namespace NS { export function inNs(){} }` and `module M {}`.** The members are extracted, but the namespace is not a scope, so the qn is `inNs`, not `NS.inNs`. Consequence in fixture f8: `NS.inNs()` is unresolved with `candidates-in-other-files` even though the definition is in the same file.
- **Ambient declarations** (`declare function ambient(...)`, all of `.d.ts`), overload signatures and class method overload signatures (`method(): void;` in a class body) are not extracted (javascript.rs:212-221 accepts `method_signature` only inside `interface_body`).
- **`enum`** produces nothing.
- **Type aliases with method members** (`type T = { z(): void }`) are deliberately not extracted (test at typescript.rs:318-319).
- **Class nodes.** cgg mints no node for a class, and does not record `FileFacts::classes` (`ClassDecl`) or `class_fields` for TypeScript. `grep local_types` across plugins finds no JS/TS hit (§05).
- **Nested qualified names** do not include the enclosing function, so same-named nested helpers in one file share a qn (fixture f10: `inner` twice).

## 03. Call-site forms

Calls are recorded by `record_call` (javascript.rs:1191-1260) for `call_expression` and `new_expression`. The callee must be either an `identifier` (bare) or a `member_expression` (receiver.method). Any other callee kind returns without recording (javascript.rs:1236).

**Supported**

- **Bare.** `helper(1)` → `RefRecord{name:"helper"}`. Fixture f2 line 31: `calls -> helper` at `high` via an import.
- **Method.** `obj.run()` → name `run`, receiver hint `obj`. Test: `call_expressions` (typescript.rs:285).
- **Deep receivers.** `a.b.c.deep()` → receiver hint `a.b.c`. Fixture f14: bound by name to the free function `deep` at `medium`.
- **Constructor.** `new Box(1)` is recorded with the name `Box` (the `constructor` field). It resolves to `Box.constructor` through step 6 (`constructor_names` → `["constructor"]`, cross_file.rs:383-391).
  - Fixture f1: `Box.make -> Box.constructor` at `medium`.
  - A class with no constructor gives `class-without-explicit-init` (fixture f2).
- **Receiver shapes are peeled** by `receiver::peel` (javascript.rs:1209-1222). The tables are in receiver.rs:60-69 and 170-205.
  - Kept as a value: `(f).m()`, `(await p).m()`, `f!.m()` (`non_null_expression`).
  - Typed: `(o as Foo).m()`, `(<Foo>o).m()` (`type_assertion`) and `new Foo().m()` set `typed_receiver = true`, receiver `Foo`.
  - Tests: `typescript_as_assertion_and_new` (receiver.rs:470) and `ts_type_assertions_parse_and_type_their_receiver` (crates/cgg/tests/resolve.rs:3177). Fixture f2 lines 24, 25 and 27 are `high`.
- **Optional chaining.** `this?.m()` → `P.m` at `high` (fixture f14), and `makeService()?.stop()` is recorded (fixture f2).
- **Chained calls.** `makeService().run()` records both calls, but the inner call's return type is **not** used for the outer one, so `run` fans out (fixture f2 line 41: `Service.run` + `Other.run`, medium).
- **Explicit generics.** `helper<number>(5)` is recorded under the bare name, read from the tree (fixture f2 line 40: `high`).
- **Tagged templates.** `` sql`select 1` `` is recorded as a call to `sql` (fixture f14: `high`).
- **IIFE body calls.** `(async () => { await later(); })()` attributes `later()` to the enclosing callable (fixture f14).
- **Decorator calls.** `@Component({...})` and `@HostListener('click')` are recorded as bare calls (fixture f1 audit: `Component`, `Input` and `HostListener` in `external_calls`).
- **Registrar arguments** (`registrar::capture`, registrar.rs:424, called at javascript.rs:1258). Gated on the call's last segment being a registrar verb of a TypeScript rule (`registrar_verbs_for("typescript")`, rules.rs ~11915).
  - The verbs are: `get`, `post`, `put`, `delete`, `patch`, `head`, `options`, `all`, `use`, `route`, `on`, `once`, `addListener`, `prependListener`, `prependOnceListener`, `Worker`, `Piscina`, `process`, the Electron `ipcMain.*`/`app.on` set, `addEventListener`, `serve`, `middy`, `Function`/`NodejsFunction`/…, the Azure/GCP/Firebase trigger verbs, `notFound`, `onError`, `registerComponent`, …, the restify `del`/`opts`/`pre`.
  - Identifier arguments become value refs (`VALUE_REF_HINT`). String arguments become string refs, with the first string being the route.
  - Fixture f7 with `--reference-edges`: `app.get('/a', h1)`, `em.on('evt', h2)` and `process.on('exit', h3)` give `reg -> h1/h2/h3` as `Via::Reference`, `medium`.
- **Arity.** `RefRecord::arity` is never set by the JS walker (`record_call` builds `..Default::default()`). TypeScript is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583, `["java","scala","cpp"]`).

**Not supported / partial**

- **Value references outside registrar verbs** are not recorded at all. The walker never calls `capture_value_refs` or `capture_value_position`; only python.rs does.
  - Fixture f7 with `--reference-edges` has no edge for `[1].forEach(cb)` or `return cb`.
  - Fixture f5: `mappedFn` (`[1].map(mappedFn)`), `timerFn` (`setTimeout(timerFn)`) and `tableFn` (`{a: tableFn}`) are reported as dead.
- **`RefRecord::value_ref`** (the site-level flag used for Java method refs and Python `return f`) is never set. `ExtractCtx::reference_sites` is not consulted by the JS walker.
- **`super(...)` constructor calls** are not recorded (the callee kind is `super`). Fixture f14 has no edge `K.constructor -> P.constructor`.
- **Element access calls** (`obj['run']()`), call-on-call (`fnReturningFn()()`'s outer call), `new (getCls())()` and dynamic `import('./m')` are not recorded as calls because the callee is not an identifier or member expression (fixtures f2 and f17). In fixture f8, `import('./lazy').then(m => m.lazyFn())` only gave `lazyFn` at `medium`, by name.
- **`helper.call(null, 1)` / `.apply` / `.bind`** are recorded as a call to `call` on receiver `helper`. They never reach `helper` (fixture f2: external `call`).
- **A local alias of a function** (`const f = helper; f(1)`) is not followed (fixture f2: `f` is external).
- **JSX elements** (`<Inner />`, `onClick={this.clicked}`) are neither calls nor value refs. Fixture f5: `Card`, `Inner` and `Old.clicked` are reported as dead. The `react-hooks` rule gap says the same (rules.rs:11478).
- **Paren-less calls**, call operators (`call_operator_names` has no JS/TS entry; cross_file.rs:394) and kwargs: not applicable or none.

## 04. Imports & module resolution

**Import forms recorded** (javascript.rs:519-934)

- `import { a, b as c } from './m'` → `from-import`, alias `a, b as c`.
- `import d from './m'` → `from-import`, alias `default as d`.
- `import * as ns from './m'` → `import`, alias `ns`.
- `import './side'` → `import` with an empty alias.
- `import type { T } from './t'` → same as a value import. Test: `import_from_captured` (typescript.rs:274).
- `import x = require('./m')` → `import`, alias `x`. Test: `import_equals_require_binds_the_module` (typescript.rs:140). Fixture f15: `lib.cjsFn()` is `high`.
- `const x = require('m')` / `const {a, b: c} = require('m')` / `x = require('m')` (CommonJS, javascript.rs:519, 563).
- **Re-exports.**
  - `export * from`, `export * as ns from` and `export { a as b } from` → `re-export` (plus an `import` record) (`record_reexport`, javascript.rs:800).
  - `export { a, b as c }` and `export default foo|function foo|class|new Api()` → `local-export`.
  - `export = value` → default export. Test: `export_assignment_is_the_default_export` (typescript.rs:154).
  - An unreadable default becomes `<value> as default` (`UNKNOWN_EXPORT`). Test: `unreadable_default_exports_are_marked_unknown`.
- **CommonJS exports.** `module.exports = …` / `exports.k = …` → `local-export`/`re-export` (`try_record_cjs_export`, javascript.rs:937). Test: `local_exports_and_commonjs_exports_are_recorded`.
- **Package records** (`FileFacts::packages`, typescript.rs:79-84, `push_package_name` javascript.rs:95):
  - the nearest `package.json` name;
  - every ancestor `deno.json`/`deno.jsonc`/`jsr.json` name and its local import-map keys;
  - the nearest `tsconfig.json`/`jsconfig.json` `compilerOptions.paths` keys, following a relative `extends` chain (bounded depth; manifest.rs:387-420);
  - `package-name` and `package-export` (from the `exports` map) import records.
  - Test: `js_names_from_package_deno_and_tsconfig` (manifest.rs:562).

**Resolution**

- **Module-scoped resolution** (`modules.rs`, JS/TS share `add_js_file`, `js_resolve` and `resolve_js`; modules.rs:266, 479, 1712).
  - **Relative specifiers** are probed with extensions in the order `ts, tsx, js, jsx, mjs, cjs, mts, cts`, then `index.*`. `./svc.js` written in a `.ts` source maps to `svc.ts` (`js_probe`, modules.rs:813-845).
  - **Workspace packages by name** resolve to `index`, `mod`, `src/index` or `src/mod`, or to a sub-path. The `exports` map sub-path names its file. Tests: `js_workspace_package_resolves_through_its_root` (cross_file.rs:12220) and `js_exports_map_subpath_names_its_file` (12970).
  - **Path aliases** (`@/`, `~`, `#`, `$`, tsconfig `paths` prefixes, Deno `/x`) are matched by path suffix with nearest-importer ranking. Test: `js_alias_prefers_the_module_nearest_the_importer` (12409). Fixture f8: `import { coreFn } from '@core/svc'` → `coreFn` at `high`.
  - **Bare specifiers** match only whole and only inside the importer's own package. Test: `js_bare_specifiers_name_packages_not_lookalike_files` (13014).
- **Barrel files.** Calls are followed through them. Fixture f2: `lib.helper(2)` and `lib.Service.create()` through `export * from './svc'` are `high`. Fixture f3: `aliasHelper` via `export { helper as aliasHelper } from './a'` is `high`.
  - Integration test: `ts_namespace_import_resolves` (crates/cgg/tests/resolve.rs:1389).
- **Cross-language imports.** TypeScript importing JavaScript resolves through the same index (fixture f15: `import { jsFn } from './lib'` with `lib.js` → `high`). JS/TS is **not** a language family (lang_family.rs:33, commented out), so an *un-imported* bare call does not reach a `.js` definition (fixture f15: `onlyJs` is unresolved).
- **External-module refusal** (`heads.rs` `WorkspaceVocab`/`FileHeads`, heads.rs:107, 236-268, 460-490, 664). A call through, or naming, an import of a non-workspace specifier goes to the external bucket and is never name-matched. Fixture f2: `axios.get` → external, and `readFile` from `'fs'` → stdlib. Fixture f8: `chunk` from `lodash` → external.
- **Stdlib heads.** `console.log`, `JSON.parse`, `Math.max`, `Promise.all` and `Object.keys` are kept out of workspace binding by stdlib-name heads (heads.rs:794). Fixture f17: `console.log` was not bound to the workspace method `Q.log`.
- **`not-in-imported-module`.** A call through an import whose module file is not in the tree is refused.
  - Fixture f12: `native.plus` via `require('./nat.node')` → `not-in-imported-module {module: './nat.node'}`.
  - Unit test: `js_module_without_a_file_is_refused` (cross_file.rs:11944).
- **Bare-name imports.** A bare call to a JS/TS name imported from a relative module is **not** tried by bare name (cross_file.rs:1666-1671); it is followed to the file instead.

**Not supported / partial**

- **A missing member of an in-tree module.** Fixture f9: `import * as lib from './svc'; lib.missing()` landed only in the audit's `external_calls` bucket with `no-candidate-in-file`. No `not-in-imported-module` record appeared in the graph's unresolved list. The cause was not traced (unverified).
- **Stale audit entries and phantom exit edges for aliased imports.** Fixtures f2 and f3: `aliasHelper()` and `h2()` (an `import { helper as h2 }`) are bound at `high`, yet also stay in the audit `external_calls` bucket. With `--include-external` they additionally get a spurious `<external>::aliasHelper` exit edge (fixture f2 `out2.json`).
- **tsconfig `paths` targets are not read**, only the keys. Aliases are matched by path suffix (CHANGELOG.md:1324-1326).
- **Dynamic `import()`** creates no module binding (fixture f8).
- **TypeScript `namespace`/`module` blocks and `declare module 'x'`** are not module scopes.

## 05. Type inference & receiver typing

TypeScript gets no plugin-recorded local types: no plugin in the JS family sets `local_types`, `scoped_types`, `class_fields` or `classes` (`grep -rln local_types crates/cgg-lang/src` lists only java, cpp, csharp, zig, python, c_macro, go, kotlin and rust). Typing therefore comes only from:

1. parameter annotations parsed from `signature_hint`;
2. receiver peeling (casts and `new`);
3. the generic "variable named like a type the file declares" heuristic;
4. return types for the return-type map.

**Supported**

- **Parameter annotations** on function declarations, methods, arrow functions and function expressions. Fixture f16: `const f = (s: Service) => s.run()` → `Service.run` at `high`, and `function (o: Other)` → `Other.run` at `high`. Fixture f2: params typed with imported classes → `high`.
- **Nullable/optional unwrapping** through `declared_type`, driven by `MAYBE_TYPE_WRAPPERS["typescript"] = ["NonNullable","Readonly"]` and `NULL_TYPE_MEMBERS["typescript"] = ["null","undefined","void"]` (stdlib.rs:120, 133).
  - `a: Foo | undefined`, `b?: Foo`, `c: null | Foo` and `e: Foo = new Foo()` all type as `Foo`. `d: Foo | Bar` stays untyped.
  - Tests: `optional_and_nullable_parameters_are_their_type` (type_hints.rs ~3419), plus annotation.rs:180-255.
  - Fixture f2: `o?: Other` and `n: Service | null = null` → `high`.
- **Return types.** `extract_return_type("typescript", "function f(): Foo | undefined")` gives `Foo`, and `Foo | Bar` gives none (type_hints.rs:3502-3512). They feed `build_return_type_map`.
- **Casts and constructions** (§03): `as`, `<T>x` and `new T()` set `typed_receiver`.
- **Lowercase-name heuristic** (`find_constructor_assignments`, type_hints.rs:2404-2461): a receiver spelled as a lower-cased type name that the **same file** declares is typed as that type. Fixture f16: `lower(service: any) { service.run() }` → `Service.run` at **`high`**, even though the parameter is `any`. This is a guess promoted to `high`.

**Not supported / partial**

- **Annotated locals.** `const s2: Service = makeService(); s2.run()` fans out to `Service.run` + `Other.run` at `medium` (fixtures f2 and f9).
- **`new`-initialised locals.** `const svc = new Service(); svc.run()` fans out (fixture f9 `ctorLocal`).
- **Return-type-initialised locals** (`const s3 = makeService()`) and chained calls (`makeService().run()`, `retChain().run()`) fan out (fixtures f2 and f9).
- **Class fields and parameter properties.**
  - `private field: Service; this.field.run()` fans out (fixture f9).
  - `constructor(private injected: Service)` followed by `this.injected.run()` fans out (fixture f9).
- **Typed closure parameters** inside calls (`[x].forEach((x: Other) => x.run())`) fan out, because the inline arrow is not a callable and TypeScript has no `scoped_types` / `closure_types.rs` support (fixture f16).
- **Generic bounds.** `function g<T extends Service>(t: T) { t.run() }` gives receiver `T` and is left unresolved (`candidates-in-other-files`, fixture f16).
- **Top types.** `any`, `unknown` and `object` are not in `TOP_TYPES` (stdlib.rs:151-156 lists only Scala, C# and Dart). Fixture f16 behaved as untyped (`a.run()` fans out), apart from the name heuristic above.
- **Missing tables and passes.**
  - No `CALLBACK_TYPE_CONSTRUCTORS` entry, so `settle_closure_args` does not apply.
  - No `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` entries (stdlib.rs:35-58).
  - No implicit receivers.
  - No `weak_receiver_type` (Python only).
  - No per-site scoping of locals: whatever propagation exists is file-wide.
- **Type information is not module-aware.** CHANGELOG.md:45-49: "Python and JS/TS record types without the module they came through, so there the type must be declared in the calling file or imported by name…".

## 06. Inheritance, dispatch & overloading

**Supported**

- **Base records.** `extends` and `implements` lists go onto each member's `base_types` (§02). The resolver derives class bases from those (dispatch.rs:121-127: only Java and C# use exact class records). TypeScript writes **no** `FileFacts::classes`.
- **`this.m()`** binds to the caller's own class first (intra-file). Inherited members resolve through the recorded bases.
  - Fixture f4: `Dog.speak -> Animal.base` via `this.base()` across files at `medium`.
  - Fixture f14: `this?.m()` → `P.m` at `high` within one file.
- **`super.m()`** resolves through the base chain. Fixture f4: `Dog.speak -> Animal.move` and `Dog.move -> Animal.move` at `medium`; `Puppy.bark -> Dog.bark` at `high` (same file as `Dog`).
- **Interface- and abstract-typed receivers** bind to the declaration. Fixture f4: `g: Greeter` → `Greeter.greet` at `high`, and `a: Animal` → `Animal.speak`/`Animal.move` at `high`.
- **Override below a typed receiver.** `d: Dog` with `d.move()` → `Dog.move` at `high`, and `d.base()` (inherited) → `Animal.base` at `medium`.
- **Abstract narrowing.** Interface and abstract members carry the `abstract` attribute, so duck-typed fan-out drops them when a concrete candidate survives. Fixture f8: `y.only()` → `Gi.only` + `Gj.only`, with the interface `G.only` dropped.
- **`--dynamic-dispatch` / `--dead-code` inheritance fan-out.** TypeScript is in `INHERITANCE_LANGS` (dispatch.rs:110-123). Fixture f4 with `--dynamic-dispatch` gives `Animal.speak -> Dog.speak/Cat.speak`, `Greeter.greet -> Dog.greet`, `Animal.move -> Dog.move` and `Dog.bark -> Puppy.bark`, all `Via::Dynamic`, `low`, `dispatch:inheritance`.
- **Overload sets.** TypeScript is in `binds_same_name_set` (intra_file.rs:835-851), so a same-file, same-qn set binds every member at `medium` rather than `ambiguous-in-file`. CHANGELOG.md:261 reports "+2,891" TypeScript edges from this. In practice the only same-qn pairs the walker emits are getter/setter pairs (`Box.size` ×2) and same-named nested helpers, because overload signatures are not extracted (§02).

**Not supported / partial**

- **No arity narrowing** (not in `OVERLOAD_ARITY_LANGS`; arity is never recorded).
- **The TypeScript `override` modifier is not recorded as an attribute.** Fixture f11: `override hook()` has attributes `None`.
- **Mixins** (`class X extends Mixin(Base)`), declaration merging, interface `extends` chains for dispatch beyond `base_types` and `implements` of an out-of-tree interface: unverified, or not modelled beyond the base-type strings.
- **`member_of_calling_class` / bare calls to inherited members.** A bare `helper()` never reaches a method: `bare_call_is_never_a_method` includes TypeScript (intra_file.rs:49-54). This is correct language semantics.
- **No extension methods and no traits.** Not applicable to the language.

## 07. Scoping & shadowing

**Supported**

- **`FileFacts::locals`** via `JS_SPEC` (locals.rs:152-181, 382). It reads parameters (including `required_parameter` and `optional_parameter`), arrow parameters, `variable_declarator`, `for…in` and `catch`. Patterns are object, array, pair, assignment and rest.
- **Lexical narrowing of nested definitions.** TypeScript is in `lexically_scoped` (intra_file.rs:119-135), applied only when locals were read (intra_file.rs:363-366).
  - Fixture f10: `outer1` and `outer2` each bind their own `inner` at `high`.
- **A local definition beats an import.** Fixture f10: `const other = () => 0; other()` binds to the local arrow callable, not to the imported `other`.
- **Locals shadowing a receiver head.** Step 0c skips module resolution when a local shadows the receiver head (cross_file.rs:5388-5390), and `external_head` checks `shadows_receiver_head` (heads.rs:683).
- **Conditional definitions** (`if (c) { function condDef(){} }`) are extracted and bind at `high` (fixture f10). They have no `DefRecord::conditional` tagging.

**Not supported / partial**

- **A parameter does not shadow an imported name.**
  - Fixture f10: `import { helper } from './a'; function p(helper: () => void) { helper(); }` produced `p -> helper` (the imported function) at **`high`**. This is a wrong edge that the shadowed-by-local rule should have prevented. Unverified why. CLAUDE.md states that locals and parameters shadow import names.
- **Out-of-scope nested definitions.** Fixture f10: `function outer3() { inner(); }`, with `inner` defined only inside `outer1` and `outer2`, bound **both** at `medium` instead of refusing.
- **No positional shadowing.** It is limited to OCaml, F# and Lua per CLAUDE.md.
- **Block scoping inside one function is not modelled.** `blocks` covers statement blocks for locals only. Unverified for `let` redeclaration in sibling blocks.

## 08. Macros, preprocessor & codegen

- **Macros, `#define` and `#if`: not applicable.** TypeScript has none of them.
- **Decorators** are the closest construct. They are recorded as attribute strings on methods and functions (`@Get(':id')`, `@HostListener('click')`), and the decorator call itself is recorded as a call (§03). Decorators on a class are recorded only as calls, because a class is not a callable.
- **Generics/templates.** Type arguments are dropped from call names (§03). No specialization records.
- **Generated-code tagging (`macro:<name>`), `name_mentions` and quote/unquote: none.** The JS walker never writes `name_mentions` (`grep name_mentions plugins/javascript.rs` finds nothing).
- **Build-time codegen** (GraphQL codegen, Prisma client, napi `index.d.ts`) is only seen if the generated files are committed. They are then ordinary TypeScript.

## 09. FFI & cross-language links

- **napi.** A Rust `#[napi]` export links to a TypeScript (or JavaScript) callable *of the same simple name* (Pass B, ffi.rs:188-260; `allowed_source_languages("napi") = ["javascript","typescript"]`, ffi.rs:694-701).
  - Fixture f12: TS `export function plus` in `index.ts` → `plus -> nat::plus`, `Via::Ffi("napi")`, `medium`.
  - This is a name-twin link between callables, not a call-site link.
  - **No snake_case→camelCase mapping**: napi's `sum_nums` is exposed as `sumNums`, which was not linked (fixture f12).
- **wasm-bindgen.** It is not language-restricted (`allowed_source_languages` returns `None`). Fixture f12: TS `wasm_fn` → `nat::wasm_fn`, `Via::Ffi("wasm-bindgen")`, `medium`. Test: `wasm_bindgen_links_rust_to_javascript` (ffi.rs:910).
- **Descriptor links.** A Protobuf `rpc` links to a TS method whose owner names the service, matched case-insensitively. Fixture f13: `Greeter.SayHello -> GreeterService.sayHello`, `Via::Descriptor("grpc")`, `low`. The match is language-agnostic (descriptor.rs:24-37, 95-104).
- **Not applicable to TypeScript:** PyO3, JNI, P/Invoke, `extern "C"` and asm.
- **Markup code-behind: none.** `markup_kinds_for` has only C# entries (rules.rs ~11813-11818). Angular templates, JSX handlers, `.vue` and `.svelte` are not read.
- **`ffi-export` framework entries** mint nodes on the Rust side only (fixture f12).

## 10. Frameworks & entry points

There are 44 built-in rules with `language: "typescript"`. Rule language is matched exactly (`frameworks/mod.rs:82-111`), so JavaScript rules do not apply to `.ts` files. Detection is by import specifier (`detect`). `node: true` mints a `<framework-entry>` node; `node: false` only marks roots. "Gap" means the rule detects the framework but enumerates no entries; its `gap` text is reported.

**Enumerating rules**

| rule id | kind | trigger | rules.rs |
| --- | --- | --- | --- |
| express | network | registrars `get post put delete patch head options all use route` | 426 |
| nestjs | network | method decorators `Get Post Put Delete Patch Head Options All MessagePattern EventPattern Sse SubscribeMessage` | 446 |
| nestjs-schedule | schedule | decorators `Cron Interval Timeout` | 476 |
| bullmq | queue | registrars `Worker process on` | 510 |
| worker-threads | queue | registrars `Worker Piscina`, plus self-module markers `parentPort`/`workerData` (rules.rs:11668) | 547 |
| electron | queue | registrars `ipcMain.handle/handleOnce/on/once`, `ipcRenderer.on/once`, `contextBridge.exposeInMainWorld`, `app.on/once` | 1996 |
| cloudflare-workers | network | registrar `addEventListener`; methods `fetch scheduled queue email trace tail` | 2100 |
| deno-http | network | registrar `serve`; method `fetch` (gap text notes limits) | 2157 |
| hono | network | registrars HTTP verbs + `use on notFound onError` | 2211 |
| aws-lambda | network | attributes `injectLambdaContext captureLambdaHandler logMetrics captureMethod`; registrar `middy`; methods `handler lambdaHandler` | 4100 |
| aws-cdk | network | registrars `Function DockerImageFunction SingletonFunction NodejsFunction GoFunction`; `string_targets: true` (`handler: "module.fn"`) | 4284 |
| azure-functions | network | registrars `http timer storageBlob storageQueue serviceBusQueue serviceBusTopic eventHub eventGrid cosmosDB` (handler in the options object) | 4441 |
| firebase-functions | network | v2 registrars `onRequest onCall onDocument* onValue* onMessagePublished onSchedule onObjectFinalized` | 4512 |
| gcp-functions | network | registrars `http cloudEvent typed` | 4810 |
| node-event-emitter | lifecycle | registrars `on once addListener prependListener prependOnceListener` (detect `events`, `node:events`, `node:process`, `stream`, …) | 6948 |
| loopback | network | decorators `get post put patch del head options` | 8458 |
| restify | network | registrars `get post put patch del head opts pre` | 8640 |
| angular | lifecycle (node:false) | base types `OnInit OnDestroy … ErrorHandler`; methods `ngOnInit … transform handleError` | 11316 |
| angular-host | lifecycle | decorator `HostListener` | 11364 |
| angular-router | lifecycle (node:false) | base types `CanActivate … HttpInterceptor`; methods `canActivate … intercept` | 11381 |
| react | lifecycle (node:false) | methods `componentDidMount … UNSAFE_componentWillUpdate` (14 names; **no `render`**) | 11414 |
| react-native | lifecycle | registrars `registerComponent registerHeadlessTask registerRunnable registerRootComponent` | 11534 |

**Gap rules** (detected, no entries): `nextjs` (581), `koa` (1733), `hapi` (1767), `fastify` (1801), `remix` (1835), `sveltekit` (1869), `nuxt` (1903), `astro` (1943), `socket-io` (2043), `trpc` (2174), `adonisjs` (8407), `feathers` (8424), `graphql-yoga` (8475), `apollo-server` (8517), `oak` (8681), `bun-serve` (8698), `react-hooks` (11478), `vue` (11556) and `svelte` (11590). The test rules `vitest`, `jest` and `playwright` are covered in §11.

**Evidence from fixtures**

- Fixture f5:
  - `express::get("/users") -> listUsers` and `express::post("/users") -> handler_at_6`.
  - `nestjs::Get(":id") -> CatsController.findOne` and `nestjs::CatsController.create`.
  - `nestjs-schedule::Cron("* * * * *") -> CatsController.tick`.
  - `angular-host::HostListener("window:resize") -> XComponent.onResize`.
  - `XComponent.ngOnInit` is rooted by `angular:base-type`, and `Old.componentDidMount` by `react:base-type`.
- Fixture f7: `node-event-emitter::on("evt") -> h2` and `on("exit") -> h3`.
- Integration tests in crates/cgg/tests/frameworks.rs: `aws_cdk_typescript_binds_a_handler_from_an_options_object` (961), `cloudflare_worker_module_handlers_are_entries` (1068), `a_closure_with_no_route_string_still_registers` (1117, `main.ts`), and the `every_cloud_platform_and_language_pair_enumerates` table (1363; the azure/gcp/aws-lambda/firebase TypeScript rows at 1272-1356).

**Helper tables**

- `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for` and `visibility_entries_for` have **no TypeScript entries** (rules.rs ~11680-11860).
- `self_module_markers_for("worker-threads","typescript")` is the only TypeScript helper entry (rules.rs:11668).

**Parity gaps against JavaScript.** These JavaScript rule ids have no TypeScript twin: `express-gateway`, `meteor`, `polka` and `sails` (compared by script over `SPECS`). TypeScript-only rules: `adonisjs`, `angular`, `angular-host`, `angular-router`, `aws-cdk`, `bun-serve`, `loopback`, `nestjs`, `nestjs-schedule`, `oak` and `trpc`.

## 11. Test harness entries

- **No TypeScript test harness enumerates.** `vitest` (rules.rs:8732), `jest` (8773) and `playwright` (8797) are `TrustKind::Test` rules with empty matchers and a `gap`. Their block verbs (`describe`, `it`, `test`, …) are deliberately not registrar verbs, because the verb set is global per language. So no `<framework-entry>::test::…` node is minted for TypeScript, and `--no-test-entries` has nothing to change.
- **Test cases are anonymous closures** passed to `it(…)` / `test(…)`, so they are not callables. Calls inside them have no enclosing callable.
  - Fixture f5 (`--include-tests`): `testHelper()` inside `it(...)` became a `toplevel:invocation` root.
  - In the same fixture, `exportedButUnused()` (imported by the test) stayed a dead-code finding, because the site had no enclosing callable and was left `no-candidate-in-file`.
- **Name-based test roots.** `is_test_name` falls back to the generic `test_` / `Test` prefix for TypeScript (roots.rs:975-990). Fixture f5: `test_named` → `builtin:test-name` test-entry root. There is no `it`/`describe` convention.
- **Test-file classification** (`testfile.rs:84-96`):
  - Names `.test.{js,jsx,ts,tsx,mjs,cjs}` and `.spec.{js,jsx,ts,tsx}`.
  - The generic directories `test`, `tests`, `__tests__`, `spec`, `specs`, `testdata`, `e2e`, `Tests`, `Test` (testfile.rs:35-45).
  - Tests: `src/a.test.ts` and `src/a.spec.tsx` are test files, and `src/attest.ts` is not (testfile.rs:197-204).
  - **Gap:** `.test.mts`, `.test.cts`, `.spec.mts`, `.spec.cts`, `.spec.mjs` and `.spec.cjs` are not in the list.
- **No `TEST_CLASS_MEMBERS_ARE_ROOTS`** (rules.rs ~11806: only python, php and ruby).
- **No `test_defs` signal.** The capability table shows `test_tagging: none` (fixture f5).

## 12. Dead-code analysis

Measured capabilities (fixture f5, `--dead-code-format json`):

```text
class: degraded, visibility: none, attributes: full, value_references: full,
dispatch: full, exports: none, test_tagging: none, max_confidence: medium
```

**Roots that apply to TypeScript**

- `framework:entry-node` and `framework:rule` (§10).
- `toplevel:invocation`: a call at module level roots its target (roots.rs:553-610). Fixture f5: `usedTopLevel`, and `listUsers` via a module-level value ref.
- `builtin:lifecycle`:
  - `CONVENTIONAL_METHODS` (`toString`, `equals`, `hashCode`, `describe`, …; roots.rs:162-170). Fixture f5: `XComponent.toString` is rooted.
  - Dunder names, since TypeScript is in `DUNDER_LANGS` (roots.rs:160-161).
  - **No TypeScript entry in `LIFECYCLE` or `LANG_ENTRY`.**
- `builtin:test-name` (§11) and `UNIVERSAL_ENTRY` `main` (roots.rs:20).
- `dispatch:inheritance` under `--dead-code` (TypeScript is in `INHERITANCE_LANGS`, deadcode/mod.rs:342).

**Signals extracted**

- Attributes and decorators.
- Value refs (registrar-argument only, §03).
- `dyn_uses` (`require` literal, §15).
- `unreachable` regions (`cfg::unreachable_after_terminator` with the `JS` spec, cfg.rs:135, 181; typescript.rs:89-90).
- Impls (via `base_types`).

**Caps and evidence**

- **Every TypeScript finding caps at `medium`** with `language-lacks-visibility`, because the plugin does not declare the `visibility` signal (javascript.rs:1356-1362 explains why: TypeScript `private` is compile-time only). Fixture f5: all 14 findings are `medium`, including `CatsController.unusedPriv` with `private-visibility` evidence.
- `public-visibility:public` evidence is recorded for class members (fixtures f5 and f11).
- `name-collides-with-screened-site:module-load` lowers the score but does not cap (fixture f5, `reflected` vs `require('reflected')`).
- The blind spot named in the report is `obj[expr] dynamic property access` (caps.rs:58).

**Not supported / partial**

- **`export` is not a root.** The `exports` signal is absent, so `export function exportedButUnused()` is reported (fixture f5, `risk: language-lacks-visibility, name-matches-unresolved-site`). The `lang:export` rule reads `FileFacts::exports`, which the JS walker never writes (roots.rs:890-915).
- **`implements-external-contract` never fires** for TypeScript. The `override` modifier is not an attribute, and the Python-style "outside base" rule is Python-only (deadcode/mod.rs:497-506). Fixture f11: `X.hook` (an `override` of the out-of-tree `Base`) is a plain `medium` finding.
- **Missing roots and refs that produce false findings:**
  - Value-ref gaps (§03): `mappedFn`, `timerFn` and `tableFn` are reported.
  - JSX gaps: `Card`, `Inner` and `Old.clicked` are reported.
  - React `render` is not in the `react` methods (fixture f5: `Old.render` is reported).
- **Unreachable regions are extracted but nothing consumes them.**
  - The only consumer of `FileFacts::unreachable` in `crates/` is the signal flag at cgg/src/lib.rs:1935; there is no reader of the regions in cgg-resolve.
  - Fixture f6: `live -> afterRet`, where the call sits after `return`, is a `high` edge, and `afterRet` is not reported.
- **Unresolved-doubt visibility filtering** is implemented only for Java and C# (deadcode/mod.rs:662-675). TypeScript `private` members get no such narrowing.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/typescript.txt`**: 594 lines (JavaScript's has 569), loaded by `stdlib_names("typescript")` (stdlib.rs:188, 229).
  - Content: globals (`console`, `JSON`, `Math`, `Promise`, `parseInt`, `setTimeout`, …), common method names (`push`, `keys`, `log`, `parse`, …) and TypeScript utility types (`Awaited`, `Exclude`, `Extract`, `NonNullable`, `InstanceType`, `ConstructorParameters`, …).
  - Diff against JavaScript: TypeScript lacks `addListener`, `bind`, `call`, `close`, `emit`, `on` and others that `javascript.txt` has, and adds the utility and iterator types.
- **Stdlib behaviour.** Calls whose bare name or receiver head is a stdlib name go to the audit's `stdlib_calls` bucket, and with `--include-stdlib` to `<stdlib>::name` exit nodes (fixture f2 `out2.json`: `<stdlib>::readFile`, `<stdlib>::setTimeout`; fixture f17: `console.log`, `JSON.parse`, `Math.max`, `Promise.all`, `Object.keys`, `parseInt`).
  - **Array literals are not stdlib.** `[1,2].map` was classified as `<external>::[1,2]::map`. `xs.push` with `xs: string[]` was external, not stdlib.
- **Type tables:**
  - `MAYBE_TYPE_WRAPPERS`: `NonNullable`, `Readonly` (stdlib.rs:120).
  - `NULL_TYPE_MEMBERS`: `null`, `undefined`, `void` (stdlib.rs:133).
  - No TypeScript entry in `UNION_TYPE_CONSTRUCTORS`, `CALLBACK_TYPE_CONSTRUCTORS`, `TOP_TYPES`, `CAST_INTRINSICS` or `CONSTRUCTING_METHODS`. `Partial<T>`, `Required<T>` and `Promise<T>` are not unwrapped (unverified for `Promise`, whose awaited value is peeled only syntactically).
- **Not applicable:** a `rust_types`-like wrapper table and implicit packages.
- **`DYN_MODULE_LOADERS`** includes `require` (facts.rs:165).

## 14. Confidence & certainty

**Paths observed at `high`**

- Intra-file single-candidate binds (fixtures f1, f5, f6, f10, f14).
- Import-anchored calls: named, default, namespace, barrel, alias and `import =` imports (fixtures f2, f3, f8, f15).
- Receivers typed by a parameter annotation, cast, assertion or `new` (fixtures f2, f16). This holds even when the type is imported rather than declared in the calling file (fixture f2 lines 11-13).
- Interface- and abstract-typed receivers binding to the declaration (fixture f4).
- `super.m()` within one file (fixture f4, `Puppy.bark`).
- The certainty rule for typed receivers in JS/TS is `in_caller_file` (cross_file.rs:4605-4606).

**What stays `medium`**

- By-name matches (unique names), duck-typed fan-out and constructors via step 6 (fixture f1).
- Inherited members and `super` across files (fixture f4).
- Dynamic-import members.
- Every value-ref (`Via::Reference`) edge.
- Overload-set binds.

**What is `low`**

- `dispatch:inheritance` edges, framework-entry edges, descriptor links and exit nodes.

**What is wrongly `high`**

- The lowercase-name heuristic (fixture f16: `service: any` → `Service.run` `high`).
- A parameter not shadowing an import (fixture f10: `p -> helper` `high`).
- Unreachable-code edges (fixture f6).

**Mermaid.** Arrows with no `high` site get a leading `?` (fixture f9 mermaid output: `N4 -->|?2x| N0`).

**Not applicable:** `weak_receiver_type`.

## 15. Dynamic use & reflection

- **dynuse coverage.**
  - `REFLECTIVE["typescript"] = ["require"]` (dynuse.rs:50). JavaScript also has `importScripts`.
  - A string literal argument of `require('x')` is captured as a `DynUse`. Since `require` is in `DYN_MODULE_LOADERS`, it is screened as `module-load`, which does **not** cap findings (fixture f5: `reflected` has `name-collides-with-screened-site:module-load` and stays `medium`).
- **No reflection screen for TypeScript:**
  - Bracket access (`o['plainTarget']()`) is not captured (fixture f11: `plainTarget` is a plain finding).
  - `Reflect.get(o, 'reflTarget')` is not captured (fixture f11).
  - `Function.prototype.call/apply/bind`, `eval`, `new Function` and dynamic `import('x')` are not captured (`import` is not in the list and is not recorded as a call, §03).
- **Blind spot disclosed** in every report: `obj[expr] dynamic property access` (caps.rs:58).

## 16. Known gaps & limitations

These were found while cataloguing. Each is cross-referenced to the section with its evidence.

1. **Locals and fields are not typed** (§05). Annotated locals, `new`-initialised locals, class fields, constructor parameter properties, return-type chains, typed inline closures and generic bounds all fall back to by-name fan-out (fixtures f2, f9, f16). This is the biggest resolution gap: the walker writes no `local_types`, `class_fields`, `classes` or `scoped_types`.
2. **Class fields holding arrow functions are not callables** (`handler = () => {}`; §02, fixture f1). Neither are object-literal arrow properties. Namespace members lose their namespace qualifier, so `NS.f()` fails to resolve (fixture f8).
3. **Value references are captured only for registrar verbs** (§03). Functions passed to `map`/`forEach`/`setTimeout` or stored in tables, and JSX handlers and components, have no reference, and are reported dead (fixtures f5, f7).
4. **No test harness for vitest/jest/playwright** (§11). Test-case closures are not callables. The `.mts`/`.cts` test suffixes are unclassified.
5. **Dead code is capped at `medium`, with no export roots** (§12). `export` is not a root, `override` is not recorded (so `implements-external-contract` never applies), React `render` is not rooted, and unreachable regions are extracted but unused.
6. **Wrong `high` edges:**
   - A parameter named like an import binds the import (fixture f10).
   - The lowercase type-name heuristic overrides an `any` annotation (fixture f16).
   - Calls after `return` stay `high` (fixture f6).
7. **Missing call shapes** (§03): `super(...)`, `obj['m']()`, `import()`, `f()()`, `.call`/`.apply`, and local function aliases are not linked.
8. **Detection** (§01): the `#!/usr/bin/env ts-node` shebang is detected as JavaScript, because detection substring-matches `node` and JavaScript is registered first.
9. **Audit and exit-node inconsistency** (§04): calls bound through aliased imports (`import { a as b }` and barrel aliases) also remain in `external_calls`, and with `--include-external` gain a phantom `<external>::name` edge (fixtures f2, f3). A missing member of an in-tree namespace import (`lib.missing()`) surfaced as external `no-candidate-in-file` instead of `not-in-imported-module` (fixture f9; cause unverified).
10. **FFI** (§09): napi name-twin linking does not convert snake_case to camelCase.
11. **Frameworks** (§10): 19 TypeScript rules (excluding the three test rules) are pure gaps, among them Next.js, Fastify, Koa, tRPC, GraphQL resolvers, Vue, Svelte and React function components. JavaScript-only rules (`express-gateway`, `meteor`, `polka`, `sails`) have no TypeScript twin.
12. **JS/TS family.** It is not a language family (lang_family.rs:33 commented out). An un-imported bare call between `.ts` and `.js` files does not bind (fixture f15).
13. **CHANGELOG follow-ups naming JS/TS:**
    - "a JS/TS path alias is matched by path suffix, not read from `tsconfig` `paths` targets" (CHANGELOG.md:1324-1326).
    - "a JS module that builds its exports at run time and an npm package the tree also has a directory named like … are refused as `not-in-imported-module` rather than external" (CHANGELOG.md:1321-1324).
    - "a JS/TS prop holding a bound action creator of the same name when the file does not import it" (CHANGELOG.md:1351-1352).
    - "TS `Log` matched by name" was reverted from `high` promotion (CHANGELOG.md:85).
14. **No ignored tests.** `grep '#[ignore'` across `crates/` returns nothing.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | full | 4 extensions, TS/TSX grammar split; `ts-node` shebang misdetected as JS |
| 02 Callables extracted | partial | functions, methods, accessors, interface/abstract members; no class-field arrows, namespace scoping, overload signatures or `declare` |
| 03 Call-site forms | partial | bare/member/new/casts/generics/optional chaining; no `super()`, `obj['m']()`, `import()`; value refs only for registrar verbs |
| 04 Imports & module resolution | full | ESM/CJS/`import =`, barrels, tsconfig/package/deno manifests, external refusal; audit residue for aliased imports |
| 05 Type inference & receiver typing | minimal | parameter annotations and casts only; locals, fields, returns and closures untyped |
| 06 Inheritance, dispatch & overloading | partial | bases, super, interface dispatch, inheritance fan-out; no `override`, no arity narrowing |
| 07 Scoping & shadowing | partial | locals and lexical narrowing; a parameter does not shadow an import (wrong `high` edge) |
| 08 Macros, preprocessor & codegen | n/a | no macros; decorators recorded as attributes and calls |
| 09 FFI & cross-language links | partial | napi/wasm-bindgen name twins (no camelCase), protobuf rpc links; no markup |
| 10 Frameworks & entry points | partial | 44 rules, 22 enumerate (Express, NestJS, Angular, cloud functions, …), 22 are gaps |
| 11 Test harness entries | minimal | test-file naming only; vitest/jest/playwright are gaps; generic `test_` name rule |
| 12 Dead-code analysis | partial | capped at `medium`; no export roots, no override contract, unreachable regions unused |
| 13 Builtins & stdlib tables | partial | 594-name stdlib list plus null/maybe tables; no top types or cast tables |
| 14 Confidence & certainty | partial | `high` for import-anchored and typed-receiver calls; some wrong `high` edges |
| 15 Dynamic use & reflection | minimal | only the `require` literal (module-load); no bracket/Reflect/import() screens |
| 16 Known gaps & limitations | n/a | 14 items listed above |
