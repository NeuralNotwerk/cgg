# 06. Inheritance, dispatch & overloading

**Category definition.** This category covers how cgg decides *which body* a call reaches when the language lets one name stand for several bodies related by a type hierarchy or by a signature. That includes: the base/supertype records a plugin emits (`FileFacts::classes` `ClassDecl`s and per-method `DefRecord::base_types`); walking those bases to find an inherited member (`resolve_via_bases`, `one_declaring_ancestor`); `super`/`base`/`parent`/`SUPER::` calls and the `super-base-out-of-graph` refusal; how a bare call inside a class is scoped to that class and its ancestry (`member_of_calling_class`, `bare_call_reaches`, `resolve_scala_bare`, the intra-file enclosing-class preference); overrides below the caller (`overrides_below`); calls typed to an interface/protocol/trait; dropping bodiless declarations from duck fan-out (`abstract` tag); the opt-in declaration → implementation edges (`--dynamic-dispatch`: `dispatch::fanout`, `dispatch::inheritance_fanout`, `cpp_virtual_dispatch`); override markers that feed dead-code's `implements-external-contract`; mixins/traits/included modules; extension methods; overload sets as one callee (`binds_same_name_set`, `one_method`); and arity narrowing (`OVERLOAD_ARITY_LANGS`, clause languages). Receiver *typing* itself is category 05; dead-code *reporting* is category 12.

Sources: section `## 06. Inheritance, dispatch & overloading` and Summary-scorecard row 06 of each `dev_docs/languages/<lang>.md`. Shared code spot-checked in `crates/cgg-resolve/src/{dispatch.rs,cross_file.rs,intra_file.rs,type_hints.rs,deadcode/mod.rs}` and `crates/cgg/src/lib.rs`. New fixtures were run with `target/release/cgg` 0.11.0 under `scratchpad/parity/matrix-06/` (full path `<scratch>/parity/matrix-06/`) and are cited as `fx:<dir>`.

## Shared machinery (what each column is driven by)

| mechanism | languages | evidence |
| --- | --- | --- |
| `INHERITANCE_LANGS` (base → override `Via::Dynamic` edges under `--dynamic-dispatch`, always under `--dead-code`; also the only languages `ClassHierarchy` is built for) | python, java, csharp, kotlin, scala, groovy, typescript, javascript, swift, dart, php, ruby | `dispatch.rs:110-123`, `168-170` |
| `CLASS_RECORDS_ARE_EXACT` (method `base_types` not unioned into the owner) | java, csharp | `dispatch.rs:130` |
| Plugins that write `ClassDecl` records at all (so a class with no methods keeps its bases) | java, csharp, kotlin, python, php (bases always empty, `php.rs:113`), rust | `grep -l ClassDecl crates/cgg-lang/src/plugins/*.rs` |
| `trait_impl_target` (drives `dispatch::fanout`) | derived from the `<Type as Trait>` qn form only, so Rust only | `cgg/src/lib.rs:649-651`, `2478` |
| `cpp_virtual_dispatch` (virtual slot → overrides, opt-in) | cpp | `cross_file.rs:2920`; gate `2899-2905` |
| `member_of_calling_class` (bare call → calling class, then bases, then `overrides_below`) | java, csharp, ruby | `cross_file.rs:566`, gate `5650` |
| `member_of_caller` (bare call beats an *external* import) | java, kotlin, scala, csharp | `cross_file.rs:5352-5353` |
| `derived_index` (feeds `overrides_below` for bare calls) | **java, csharp only** | `cross_file.rs:424-430` |
| `bare_call_reaches` scoping | never-a-member: python, rust, js, ts, php, fsharp, lua; class+ancestry: dart, ruby; package: go, perl; module+imports: elixir; Java/C#/Scala via WS14 paths; unchanged: kotlin, groovy, cpp, swift, objc and every other language | `cross_file.rs:4063-4160` |
| `bare_call_is_never_a_method` (intra-file) | python, rust, go, javascript, typescript, php, lua | `intra_file.rs:49-54` |
| `super_owner_may_be_anonymous` (super is never `high`) | scala, kotlin, groovy | `cross_file.rs:4697-4699` |
| `abstract`-tagged declarations (dropped from duck fan-out when a concrete candidate survives) | python (`@abstractmethod`, `Protocol`/`ABC` owners), typescript (`javascript.rs:370`), swift, scala, dart | `cross_file.rs:1153-1190`; `grep '"abstract"' plugins/*.rs` |
| `OVERRIDE_MARKERS` (dead-code `implements-external-contract`) | matches `@Override`, `override`, `@override` only; in practice recorded by java, groovy, csharp (fx:ov) | `deadcode/mod.rs:409`, `500` |
| `binds_same_name_set` (same file + same qn = one callee) | java, cpp, csharp, kotlin, scala, swift, groovy, fsharp, solidity, typescript, julia + clause languages | `intra_file.rs:836-855` |
| `one_method` (overload set exempt from `--fanout-cap`) | `binds_same_name_set` minus clause languages | `cross_file.rs:4787-4805` |
| `OVERLOAD_ARITY_LANGS` (`accepts_arity`, typed-lookup overload expansion) | java, scala, cpp | `type_hints.rs:1583`, `1590`; `cross_file.rs:2621`, `4817` |
| `is_clause_language` (first clause per arity) | erlang, elixir, haskell (Haskell arity forced to 0) | `intra_file.rs:857-860`, `891`, `941`; cross-file only for elixir/erlang `cross_file.rs:5549`, `5600` |

## Legend

| mark | meaning |
| --- | --- |
| ✅ | full: exists and covers the language's common cases |
| 🟡 | partial: exists but misses a common case, stays medium where a peer reaches high, or yields wrong edges |
| ⬜ | none: the concept exists in the language, cgg does nothing |
| — | n/a: the concept does not exist in the language (or is correctly not modelled, e.g. Python bare calls never reach methods) |
| ❔ | not confirmed by code or fixture (treat as unverified) |

### Columns

| column | capability |
| --- | --- |
| **Bases** | Base/supertype/conformance/role records exist for the language's inheritance construct |
| **Chain∅** | A class with **no methods** keeps its bases (needs a `ClassDecl` or equivalent), so `Leaf → Mid{} → Base` still reaches `Base.m` |
| **InhTyped** | A receiver typed as the subclass reaches an inherited member (`resolve_via_bases`) |
| **InhSelf** | `self.m()` / `this.m()` / `$this->m()` / `$self->m` / implicit-this bare call reaches an inherited member |
| **Super** | `super`/`base`/`parent::`/`SUPER::` call resolves through the base chain (and is refused `super-base-out-of-graph` when the chain leaves the tree) |
| **BareScope** | A bare call inside a class is limited to that class, its ancestry and module-level functions (never an unrelated class's member) |
| **OvrBelow** | In the *default* graph, a call through the caller's own class also reaches overrides in derived classes (`overrides_below`) |
| **IfaceDecl** | A receiver typed as an interface/protocol/trait/abstract type binds the declaration |
| **AbsNarrow** | Bodiless interface/abstract/protocol declarations are tagged and drop out of duck fan-out when a concrete candidate survives |
| **DynFan** | `--dynamic-dispatch` (and `--dead-code`) add declaration → implementation/override edges |
| **OvrMark** | The override marker is recorded and matched by `OVERRIDE_MARKERS` (or an equivalent, e.g. Rust `trait_impl_target`), so `implements-external-contract` can fire |
| **Mixin** | Mixins / traits / included modules / roles / embedding contribute members through the base chain |
| **Ext** | Extension methods / categories / type extensions bind on a receiver of the extended type |
| **OvlSet** | Same-qn overloads (or dispatch methods / clauses) are one callee, bound together, never cap-refused, and a typed cross-file lookup keeps the whole set |
| **Arity** | The call's argument count narrows an overload or clause set |
| **Other** | Language-specific dispatch (multimethods, S3/S4, type classes, protocols, behaviours, vtables, templates, generic-trait overloads) |

## The matrix

| lang | Bases | Chain∅ | InhTyped | InhSelf | Super | BareScope | OvrBelow | IfaceDecl | AbsNarrow | DynFan | OvrMark | Mixin | Ext | OvlSet | Arity | Other | notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | ✅ | — | ✅ | ✅ | — | — | — | ✅ | 🟡 | ✅ | ✅ | — | ✅ | — | — | ✅ | Trait method called through a generic bound (`t: &T`) is not dispatched; trait declarations are not tagged `abstract`, so duck fan-out keeps them next to impls (only the over-cap homogeneous case binds the declaration) |
| python | ✅ | ✅ | ✅ | 🟡 | ✅ | — | ⬜ | ✅ | ✅ | ✅ | 🟡 | 🟡 | — | 🟡 | — | ⬜ | Cross-file inherited `self.m()` is unresolved (`candidates-in-other-files {1}`, fx:py) while `super().m()` in the same file pair binds high |
| kivy | ⬜ | — | — | ⬜ | — | — | — | — | — | — | — | ⬜ | — | — | — | — | `<A@Button+Mixin>` bases are parsed by the grammar but never read (`kivy.rs:155`); KV → Python owner match has no base walk (`ffi.rs:450-484`, inferred) |
| javascript | 🟡 | ⬜ | 🟡 | 🟡 | 🟡 | — | ⬜ | — | — | ✅ | — | ⬜ | — | — | — | ⬜ | Bases only on method records: an empty intermediate class breaks the chain and `this.m()` fans out to an unrelated class (fx:empty/js); prototype/`Object.assign` inheritance not modelled |
| typescript | 🟡 | ⬜ | 🟡 | 🟡 | 🟡 | — | ⬜ | ✅ | ✅ | ✅ | ⬜ | ⬜ | — | 🟡 | ⬜ | — | Empty intermediate class breaks typed lookup (fx:empty/ts unresolved); `override` modifier not recorded (fx:ov); overload signatures not extracted so arity cannot narrow |
| go | ⬜ | — | ⬜ | — | — | — | — | ✅ | ⬜ | ⬜ | — | ⬜ | — | — | — | — | No embedding (promoted methods bind by name, medium) and no interface → implementation edges: `--dynamic-dispatch` adds nothing |
| java | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | ✅ | — | — | ✅ | ✅ | 🟡 | Interface/abstract members not tagged `abstract` (fx:ov `I.m` attrs `None`); cross-file arity narrowed to one stays medium; enum-constant receivers external |
| kotlin | ✅ | ✅ | ✅ | 🟡 | 🟡 | 🟡 | ⬜ | ✅ | ⬜ | ✅ | ⬜ | ⬜ | ✅ | ✅ | ⬜ | — | No `member_of_calling_class`: bare inherited `shared()` with an unrelated same-named member elsewhere is refused (fx:kt); no arity; `override` modifier not recorded (fx:ov) |
| c | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | Function-pointer "vtables" (`ops->run()`) not modelled |
| cpp | 🟡 | ⬜ | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | ⬜ | 🟡 | ⬜ | — | — | ✅ | ✅ | 🟡 | Empty intermediate struct breaks the chain and binds an unrelated `Other::m` (fx:empty/cpp); `override`/`final`/`= 0` not recorded; qualified non-virtual calls still get virtual fan-out |
| csharp | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⬜ | ✅ | ✅ | — | 🟡 | ✅ | ⬜ | ⬜ | No arity narrowing although C# overloads as much as Java; interface/abstract members join duck fan-out (fx:ov `U.Use → I.M, B.M, C.M, D.M`) |
| bash | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Redefinition binds both copies at medium (no last-wins) |
| ruby | 🟡 | ⬜ | ✅ | 🟡 | ⬜ | ✅ | ⬜ | — | — | ✅ | — | ✅ | — | — | — | ⬜ | `super` draws no edge; no `ClassDecl` so `class Empty < Base; end` loses its base; intra-file ignores ancestry (wrong high, ruby.md fixture p3) |
| swift | 🟡 | ⬜ | ✅ | 🟡 | ✅ | ⬜ | ⬜ | ✅ | ✅ | 🟡 | ⬜ | 🟡 | 🟡 | 🟡 | ⬜ | — | Typed cross-file overload lookup keeps only the last-declared overload, so `o.f(1)` binds `f(_:_:)` (fx:sw); method-less types lose bases |
| lua | ⬜ | — | ⬜ | ⬜ | — | — | — | — | — | ⬜ | — | ⬜ | — | — | — | — | `setmetatable(Dog, {__index = Animal})` not interpreted |
| php | 🟡 | ⬜ | ❔ | 🟡 | ✅ | — | ⬜ | ✅ | ⬜ | ✅ | ⬜ | 🟡 | — | — | — | ⬜ | Method-less intermediate class breaks `$this->m()` (fx:empty/php, `candidates-in-other-files {2}`); `#[\Override]` is captured but never matches `OVERRIDE_MARKERS` (fx:ov) |
| dart | 🟡 | ⬜ | ✅ | ✅ | ✅ | 🟡 | ⬜ | ✅ | ✅ | ✅ | ⬜ | ✅ | ⬜ | — | — | — | Empty intermediate class: bare `m()` binds both `Base::m` and unrelated `Other::m` (fx:empty/dart); `@override` not captured; no extension-method index |
| scala | ✅ | 🟡 | 🟡 | ✅ | 🟡 | ✅ | ⬜ | 🟡 | ✅ | ✅ | ⬜ | 🟡 | ⬜ | ✅ | ✅ | ⬜ | Typed `l.m()` through an empty intermediate class is refused though the bare call binds (fx:empty/scala); no trait linearization; Scala 3 `extension` / implicit classes not linked |
| hcl | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Nothing to model |
| zig | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | ⬜ | `self.vtable.f(self.ptr)` interface pattern and `usingnamespace` not modelled |
| objc | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | — | — | 🟡 | ⬜ | — | — | No `@interface C : A` bases: `[self run]` inherited cross-file is unresolved, `[super ping]` fans out to unrelated classes |
| r | ⬜ | — | ⬜ | ⬜ | ⬜ | — | — | — | — | ⬜ | — | — | — | — | — | ⬜ | No S3 generic → `area.circle` link, no S4 `setMethod`, no R6 `inherit` |
| groovy | 🟡 | ⬜ | ✅ | 🟡 | 🟡 | ⬜ | ⬜ | ✅ | ⬜ | ✅ | ✅ | ⬜ | ⬜ | 🟡 | ⬜ | ⬜ | Bare calls unscoped (wrong same-file highs); typed cross-file overload lookup keeps one overload (fx:ovx/gv, new); empty intermediate class breaks chain (fx:empty/groovy) |
| julia | ⬜ | — | ⬜ | — | — | — | — | ⬜ | — | ⬜ | — | — | — | ✅ | ⬜ | ⬜ | Multiple-dispatch methods bind as one set at medium; no arity/type method selection; `<:` hierarchy not recorded |
| perl | ✅ | ✅ | ⬜ | ✅ | 🟡 | — | ⬜ | — | — | ⬜ | — | ✅ | — | — | — | ⬜ | Has full `@ISA` records but is not in `INHERITANCE_LANGS`, so no override edges; non-invocant `$obj->m` never walks bases |
| elixir | — | — | — | — | ⬜ | — | — | ⬜ | — | ⬜ | — | 🟡 | — | ✅ | 🟡 | ⬜ | Imported bare calls ignore arity (fixture f1 binds `fmt/2` for `fmt(1)`); behaviours/protocols not dispatch |
| erlang | — | — | — | — | — | — | — | ⬜ | — | ⬜ | — | — | — | ✅ | 🟡 | — | Arity mismatch falls back to every arity (one false high); `-behaviour`/`-callback` not linked |
| fortran | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | ⬜ | — | — | — | ⬜ | ⬜ | ⬜ | `type, extends(base)`, deferred bindings and generic interfaces all unmodelled; interface bodies compete with bodies |
| clojure | ⬜ | — | — | — | — | — | — | ⬜ | — | ⬜ | — | — | — | ✅ | — | 🟡 | Protocol methods are not callables (calls go external); multimethods fan out to every `defmethod` only by shared qn |
| haskell | — | — | — | — | — | — | — | ⬜ | — | ⬜ | — | — | — | 🟡 | — | ⬜ | Type-class instances merged as clauses of one function (wrong high); cross-file binds the *last* equation |
| ocaml | ⬜ | — | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | ⬜ | — | — | — | — | — | ⬜ | Classes (`inherit`) not extracted; module signatures / functors not linked |
| powershell | ⬜ | — | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | — | `class K : Base` bases unread; `: base()` breaks the parse; same-file overloads `ambiguous-in-file` |
| solidity | ⬜ | ⬜ | ⬜ | 🟡 | 🟡 | 🟡 | ⬜ | ❔ | ⬜ | ⬜ | ⬜ | — | ✅ | ✅ | ⬜ | ⬜ | `contract C is A` bases ignored: inherited calls bind by import luck and reach unrelated contracts; no C3 linearization |
| fsharp | ⬜ | — | ⬜ | 🟡 | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | ❔ | — | ⬜ | ✅ | ⬜ | — | `base.M()` fans out to sibling classes (wrong edges); `inherit` / `interface … with` unread; abstract members not nodes |
| starlark | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Nothing to model |
| cmake | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | "Last definition wins" and `_name()` to the overridden command not modelled |
| nix | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | Overlays / `lib.fix` / module merges not modelled |
| verilog | ⬜ | — | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | ⬜ | — | — | — | — | — | — | SystemVerilog `extends`, `virtual`, `super`, interface classes unmodelled (classes are not even owners) |
| vhdl | — | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | ⬜ | ⬜ | Type-resolved subprogram overloads extracted but not grouped (not in `binds_same_name_set`); architecture/configuration choice not modelled |
| asm | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Nothing to model |
| smithy | — | — | — | — | — | — | — | — | — | — | — | 🟡 | — | — | — | — | `with [Mixin]` is a plain reference edge, members not inherited |
| proto | — | — | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | proto2 `extend` not modelled; rpc → host implementation is a descriptor link (cat. 09) |
| graphql | 🟡 | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | — | — | `implements` / union membership are forward reference edges only; no interface → implementer edges |
| openapi | 🟡 | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | `allOf`/`oneOf`/`anyOf` are plain `$ref` edges |
| asyncapi | 🟡 | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | `allOf` is a plain `$ref` edge; message `traits` are not nodes |
| lean | ⬜ | — | ⬜ | — | — | — | — | 🟡 | — | ⬜ | — | — | — | — | — | ⬜ | `structure P extends Q` unrecorded; typeclass call binds the class field, never the instance |

### Cell evidence (non-obvious cells)

- **rust** — Bases `rust.rs:563-566`; trait defaults consulted in step 4a `cross_file.rs:6668-6700`; `homogeneous_trait_declaration` `cross_file.rs:273`; `narrow_trait_overloads` `cross_file.rs:5041` (Other); DynFan `dispatch.rs:38` driven by `trait_impl_target` (`cgg/src/lib.rs:2478`); OvrMark = `trait_impl_target` read by `Contracts::external_contract` (`deadcode/mod.rs:472-485`); Ext = test `rust_value_typed_as_container_reaches_its_extension_trait_impl`; AbsNarrow 🟡 because a trait declaration carries no `abstract` tag (`grep '"abstract"' rust.rs` = none) and rust.md's fixture `unknown_thing.put()` bound `Backend::put` with both impls.
- **python** — Chain∅ fx:empty/py `b.u -> a.Base.m` (medium) through bodiless `class Mid(Base): pass`; InhSelf fx:py `pkg.sub.S1.go inh self candidates-in-other-files {1}` vs `S1.go2 -> pkg.base.Plain.inh high` via `super()`; OvrMark 🟡: Python records no marker, `external_contract` treats any method of a class with an outside base as a contract (`deadcode/mod.rs:500-506`); Mixin 🟡 no C3 MRO (python.md, `one_declaring_ancestor` refuses high, `cross_file.rs:4654`); OvlSet 🟡: `@overload` stubs are not dropped and Python is not in `binds_same_name_set` (`intra_file.rs:836-855`).
- **javascript / typescript** — no `ClassDecl` writer; fx:empty/js `Leaf.z -> Base.m` **and** `Leaf.z -> Other.m` (medium); fx:empty/ts `u m Leaf candidates-in-other-files {2}`; TS `override` fx:ov `C.m None`; Super 🟡 = cross-file `medium` (javascript.md f5, typescript.md f4).
- **java** — Chain∅ test `java_chain_runs_through_a_class_with_no_methods` (`dispatch.rs:730`); OvrBelow fx:jv `p.Use.run -> p.Base.hook` + `p.Sub.hook` (medium); AbsNarrow fx:ov (Java interface and abstract members have no `abstract` attribute); OvrMark fx:ov `C.m ['@Override']`.
- **kotlin** — Chain∅ fx:empty/kt `p.u -> p.Base.m high`; IfaceDecl fx:empty/kti `p.u -> p.I.m high`; BareScope 🟡 fx:kt `p.Leaf.start shared candidates-in-other-files {2}` (gate `cross_file.rs:5650` excludes Kotlin; only the external-import path `5352-5353` includes it); Ext step 0b `kotlin_extension_in_scope` `cross_file.rs:3611`; OvrMark/AbsNarrow fx:ov all Kotlin members `attributes None`.
- **cpp** — fx:empty/cpp `u -> Base::m` and `u -> Other::m` (medium) through `struct Mid : Base {}`; DynFan 🟡 qualified call fan-out (cpp.md fixture f5); Other 🟡: template specializations (`narrow_specializations`) and member-pointer tables work, CRTP does not.
- **csharp** — Chain∅ fx:empty/cs `N.U.Us -> N.Base.M high`; AbsNarrow fx:ov duck call bound `I.M` and `B.M` (both declarations) beside `C.M`, `D.M`; Ext 🟡 never `high`, untyped field receivers miss (csharp.md f8); Arity ⬜ fx:ovx/cs `o.F(1)` and `o.F(1, 2)` each bound both overloads (medium).
- **ruby** — OvrBelow ⬜: `member_of_calling_class` runs for Ruby (`cross_file.rs:5650`) but `derived_index` is built for java/csharp only (`cross_file.rs:428-430`), so `overrides_below` is always empty; Chain∅ fx:empty/rb produced no edge and no audit record for paren-less `m` (paren-less sites leave no record when they bind nothing, CLAUDE.md step 3).
- **swift** — fx:sw `use -> Over.f (func f(_ a: Int, _ b: Int))` for **both** `o.f(1)` and `o.f(1, 2)`; cause: whole-set expansion gated on `OVERLOAD_ARITY_LANGS` (`cross_file.rs:2619-2623`); fx:empty/sw unresolved; DynFan 🟡 static requirements not fanned (swift.md f12).
- **php** — fx:empty/php `Leaf::z m this candidates-in-other-files {2}`; fx:ov `C::m ['#[\\Override]']`, never equal to any `OVERRIDE_MARKERS` entry after `trim()` (`deadcode/mod.rs:409`, `500`); InhTyped ❔ php.md only covers `$this`/`self`/`static`/`parent` and an interface-typed parameter.
- **dart** — fx:empty/dart `Leaf::z -> Base::m` **and** `Leaf::z -> Other::m`, plus typed `u m Leaf` refused; BareScope 🟡 for that reason; OvrMark fx:ov `C::m None` (Dart declares no `attributes: true`).
- **scala** — fx:empty/scala bare `Leaf::z -> Base::m` (medium) but `U::u m Leaf candidates-in-other-files {2}`; IfaceDecl 🟡 a typed abstract receiver binds the concrete member instead of the declaration (scala.md f1); fx:ovx/sc arity picks the right overload per site (medium).
- **groovy** — fx:ovx/gv: `o.f(1)` and `o.f(1, 2)` both bound only `void f(int a, int b)`; fx:gv bare `inherited()` refused (2 candidates); OvrMark fx:ov `C.m ['@Override']`.
- **perl** — Chain∅ ✅ because `perl-isa` records are per package, not per method (`perl.rs:259-300`); DynFan ⬜ not in `INHERITANCE_LANGS` (`dispatch.rs:110-123`).
- **solidity** — Ext ✅ = `using L for T` step 4e; IfaceDecl ❔ (solidity.md shows `IVault.deposit` gets no dyn edge but does not test a typed interface call).
- **fsharp** — OvrMark ❔ (fsharp.md does not say whether `override`/`default` are recorded; F# declares no `attributes: true`).

## Parity gaps

Ranked by impact on graph correctness (wrong edges first, then missing edges), then by how many languages share the gap.

1. **Method-less classes lose their bases (no `ClassDecl`)** — TypeScript, JavaScript, Dart, Swift, Groovy, Ruby, PHP, C++ (and Scala on the typed path), while Java, C#, Kotlin and Python keep them. The result is not only a miss: Dart, JS and C++ bind the unrelated `Other.m` (fx:empty/dart, fx:empty/js, fx:empty/cpp). PHP writes `ClassDecl` but with empty bases (`php.rs:113`) and fills `PhpIndex.bases` from definitions only (`libscope.rs:1971-1985`). Evidence: fx:empty/*, `dispatch.rs:171-182` reads `f.classes`. Effort **M** (one `ClassDecl` push per plugin, as `python.rs:211-223` and `kotlin.rs:288-298` do; PHP also needs `PhpIndex` to read classes).
2. **Typed cross-file overload lookup keeps one arbitrary overload** — Swift and Groovy bind every arity to the last-declared overload, a wrong edge, while Kotlin and C# bind the whole set and Java/Scala/C++ narrow it (fx:sw, fx:ovx). Cause: the set expansion at `cross_file.rs:2619-2623` requires `OVERLOAD_ARITY_LANGS`. Effort **S** (expand for every `binds_same_name_set` language, not only arity languages).
3. **No arity narrowing in overloading languages other than Java/Scala/C++** — C#, Kotlin, Swift, Groovy, TypeScript (overload signatures), Solidity, F#, PowerShell, VHDL. Each binds every overload at medium (fx:ovx/cs, kotlin.md f1, groovy.md f1, solidity.md f1). Evidence `type_hints.rs:1583`; plugins set no `RefRecord::arity`. Effort **M** per language (record arity in the plugin; check `accepts_arity` reads its signature syntax: C# `params`, Kotlin `vararg`/defaults, Swift labels/defaults). C# and Kotlin first.
4. **Wrong-owner highs from unscoped bare / `this` calls** — Groovy binds a bare call to an unrelated class's method at high (groovy.md f13), PHP binds `$this->x()` to another class in the same file at high (php.md p10), Ruby intra-file ignores ancestry (ruby.md p3), Swift same-file overridden bare call is `ambiguous-in-file`. Java/C#/Dart/Scala apply class scope. Evidence `cross_file.rs:4078-4085` lists Kotlin, Groovy, C++, Swift, ObjC as "unchanged". Effort **M** (extend `member_of_calling_class` and the intra-file enclosing-class preference to Groovy, Kotlin and Swift; teach intra-file the PHP `$this` owner rule).
5. **Python cross-file inherited `self.m()` is unresolved** — fx:py shows `candidates-in-other-files {1}` though `super().m()` from the same file binds high and Java/Kotlin/C# bind the same shape. Root cause unverified (python.md §06). Effort **M**.
6. **`overrides_below` exists only for Java/C#** — `derived_index` (`cross_file.rs:424-430`) skips Ruby even though Ruby runs `member_of_calling_class`, and no other `INHERITANCE_LANGS` language (Python, Kotlin, Scala, Dart, Swift, TS, JS, Groovy, PHP) reaches overrides below `self`/`this` in the default graph. Only `--dynamic-dispatch` links them. Effort **S** for Ruby (add to `derived_index`), **M** to route `self.`/`this.` calls through it.
7. **Interface / abstract declarations not tagged `abstract`** — Java, C#, Kotlin, Groovy, PHP, C++ (`= 0`), ObjC protocols, Fortran interface bodies, Rust trait declarations, while TS, Swift, Scala, Dart and Python tag them. Duck fan-out therefore keeps the declaration as an extra target (fx:ov C# `I.M`, `B.M`). Effort **S** per plugin (push `"abstract"` on bodiless/`abstract`/interface members).
8. **Override markers missing or unmatched** — Kotlin, Swift, Scala, TypeScript, C++ (`override`), Dart (`@override`), Solidity (`override_specifier`) record nothing (fx:ov), and PHP's `#[\Override]` is recorded but never matches `OVERRIDE_MARKERS` (`deadcode/mod.rs:409`). Java, C# and Groovy work. So `implements-external-contract` never caps their overrides of outside contracts. Effort **S**.
9. **Languages with real inheritance but no base records** — Objective-C (`@interface C : A`, protocols), PowerShell (`class K : Base`), Solidity (`is A`), F# (`inherit`), Lua (metatables), Julia (`<:`), Fortran (`extends`), OCaml (`inherit`), SystemVerilog (`extends`), Lean (`extends`), R (S4 `contains`, R6 `inherit`), Kivy (`@Base+Mixin`). ObjC `super` and F# `base.` then fan out to sibling classes, which are wrong edges. Effort **M** each (ObjC, Solidity and F# first because they produce wrong edges).
10. **Perl has bases but no override edges** — full `@ISA`/`parent`/`extends`/`with` records and DFS resolution, but Perl is not in `INHERITANCE_LANGS` (`dispatch.rs:110-123`), so `--dynamic-dispatch`/`--dead-code` never link `My::Domain::greet` to `My::Domain::Void::greet`. Also `$class->SUPER::new` without an implicit `Moose::Object` base goes external. Effort **S–M** (feed `perl-isa` records into `ClassHierarchy`, add to the list).
11. **`super` handling outside the base-chain languages** — Ruby `super` draws no edge; Kotlin, Scala, Groovy `super` are never high (by design: `cross_file.rs:4697-4699`); Solidity and F# `super`/`base` bind by name. Python, Java, C#, Swift, Dart, PHP (`parent::`) and Perl (`SUPER::`) reach high. Effort **M** for Ruby (zsuper has no name at the site; needs the enclosing method's name).
12. **Extension methods** — C# (step 4d) and Kotlin (step 0b) have an index; Dart `extension … on T`, Scala 3 `extension`/implicit classes, F# type extensions and Groovy extension modules bind by name only or not at all, and Swift extensions miss literal receivers. Effort **M**.
13. **Mixin semantics** — Scala trait linearization, Python C3 MRO, PHP `insteadof`/`as`, Ruby refinements and `module_function`, Groovy traits/`@Mixin`, Kotlin class delegation `by`, Go embedding and Zig `usingnamespace` are absent. Dart `with` and Ruby `include`/`extend`/`prepend` work as plain bases. Effort **M–L**.
14. **Interface → implementation in structurally or nominally dispatched non-OO languages** — Go interfaces (structural), Haskell type classes (instances merged wrongly), Clojure protocols/records, Elixir protocols/behaviours, Erlang behaviours, Lean instances, R S3/S4 generics, Julia methods extending `Base`. Rust gets this through `trait_impl_target` and C++ through `cpp_virtual_dispatch`. Effort **L**.
15. **Clause-language arity holes** — Elixir imported bare calls ignore arity (elixir.md f1/f5); Erlang `-import` path ignores arity and an arity mismatch falls back to every arity, producing a false high (erlang.md f2); Haskell cross-file calls pick the *last* equation because `first_clause_per_arity` runs only for Elixir/Erlang (`cross_file.rs:5549`, `5600`). Effort **S** each.
16. **C++ specific** — a qualified non-virtual `Base::draw()` still gets virtual fan-out (cpp.md f5; seed loop `cross_file.rs:3043-3093`), and C++ is outside `INHERITANCE_LANGS`, so dead-code `overrides_below` evidence never runs (`deadcode/mod.rs:342`). Effort **S**.
17. **Redefinition semantics** — Bash, R, Clojure, CMake, PowerShell use "last definition wins" (CMake also `_name`), but cgg either binds every copy at medium (Bash) or refuses `ambiguous-in-file` (R, Clojure, CMake, PowerShell). Effort **S–M**.

## Doc corrections

- **kotlin.md §06, "`member_of_calling_class`"** cites `cross_file.rs:5348-5375`. That range is the external-import `member_of_caller` path (`cross_file.rs:5352-5353`, java/kotlin/scala/csharp). The real `member_of_calling_class` gate is `matches!(lang, "java" | "csharp" | "ruby")` (`cross_file.rs:5650`). With an in-tree same-named member elsewhere, Kotlin's bare inherited call is refused: fx:kt `p.Leaf.start shared candidates-in-other-files {2}`. The doc's f2 result is high only because the name was unique.
- **kotlin.md §06, "Overrides below a base member … This is the override fan-out"** — `overrides_below` is fed by `derived_index`, which is java/csharp only (`cross_file.rs:428-430`). The two medium edges are the generic same-name candidate binding, not override handling (exact path unverified, but it cannot be `overrides_below`).
- **ruby.md §06, "That includes overrides below the calling class and the fan-out cap on them"** — wrong for Ruby. `derived_index` skips Ruby (`cross_file.rs:428-430`), so `overrides_below` returns nothing. The doc's own fixture p3 note (`Derived::step` reached only by the dyn edge) agrees with the code.
- **groovy.md §06, "Overload sets … bind every overload at medium"** — true intra-file only. A typed cross-file call keeps one overload (the last declared) for every arity: fx:ovx/gv bound `o.f(1)` and `o.f(1, 2)` both to `void f(int a, int b)`. Same cause as the Swift bug swift.md already documents (`cross_file.rs:2619-2623`).
- **typescript.md, javascript.md, dart.md, scala.md, groovy.md, cpp.md §06** do not record that an intermediate class with no methods breaks the base chain (fx:empty/{ts,js,dart,scala,groovy,cpp}). For JS, Dart and C++ it creates a wrong medium edge to an unrelated class, and for Scala only the typed path is affected (the bare `resolve_scala_bare` path binds).
- **csharp.md §06** does not mention that interface and abstract declarations stay in duck fan-out (no `abstract` tag): fx:ov `U.Use -> I.M, B.M, C.M, D.M`. The same holds for Java and Kotlin (fx:ov attributes `None`).
- **php.md §06/§12**: `#[\Override]` attributes are captured (`C::m ['#[\\Override]']`, fx:ov) but cannot match `OVERRIDE_MARKERS` (`deadcode/mod.rs:409`), so PHP overrides never get `implements-external-contract`.
- **Code comment, not a language doc**: `deadcode/mod.rs:405-408` says `OVERRIDE_MARKERS` covers "Dart's `@override`", but the Dart plugin does not capture annotations (no `attributes: true` in `dart.rs`; fx:ov `C::m None`). The CLAUDE.md "a bare call to a member the caller's class declares or inherits beats an import (Java/Kotlin/C#)" is likewise true for Kotlin only when the competing import is external (`cross_file.rs:5352`).
- **cpp.md** correctly flags that the `cpp_virtual_dispatch` doc comment (`cross_file.rs:2915-2918`) is stale against the opt-in gate. This is noted here only so it is not lost.
- **swift.md** says the cross-file comment at `cross_file.rs:4086-4088` ("Swift, Objective-C, whose plugins record no base types") is stale for Swift. Confirmed: Swift records `base_types` (`swift.rs:333-350`). The comment is still right for Objective-C.
