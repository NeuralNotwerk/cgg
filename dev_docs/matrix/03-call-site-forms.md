# 03. Call-site forms

**Definition.** A *call-site form* is a syntactic shape in source that a language plugin turns into a `RefRecord` (a reference from the enclosing callable to a name), together with the flags it attaches to that record. Those flags are `receiver_hint`, `typed_receiver`, `path_receiver`, `arity`, `kwargs`, `paren_less`, `value_ref`, `implicit_receiver`, `invocant`, `arrow` and `arg_type`. This category covers which shapes are *recorded* and *what the record carries*: bare, member, qualified/path, chained, constructor, explicit generics, operators and call operators, paren-less calls, accessors, value references and callbacks, registrar arguments, arity and keyword arguments, implicit receivers, and calls embedded in strings or macros. How a recorded site is later *bound* belongs to sections 04–07. This matrix still marks a form ⬜ or 🟡 when it is recorded in a shape that can never bind, for example Julia's `Mod.f` recorded as one dotted name, or Java's `new Box<String>` recorded under the name `Box<String>`. For the interface languages (Smithy, Protobuf, GraphQL, OpenAPI, AsyncAPI), the "call" is a type or `$ref` reference. Their **Bare** column means "a plain reference is recorded" and **Qual** means "a qualified/pointer reference keeps enough of its path to bind correctly".

Sources: section 03 and scorecard row 03 of every `dev_docs/languages/<lang>.md`, then spot-checks against the code and the fixtures listed at the end. Where a language doc and the code or a fixture disagreed, the matrix follows the code or fixture (see **Doc corrections**).

## Legend

| mark | meaning |
| --- | --- |
| ✅ | full: the form is recorded and carries what the resolver needs |
| 🟡 | partial: recorded in some shapes only, recorded in a shape that rarely binds, or wired but inert |
| ⬜ | none: the language has the concept, but cgg records nothing usable for it |
| — | n/a: the concept does not exist in the language, or is not a separate syntactic form there |
| ❔ | unverified: no language doc covers it and no fixture was run, so it is left ungraded |

## Columns

| column | what full support means | reference implementation | how it was checked across plugins |
| --- | --- | --- | --- |
| **Bare** | `f(x)` (or the language's plain application or command) is recorded as a name with an empty receiver | every plugin | §03 of each doc |
| **Member** | `recv.m()` / `recv->m()` / `[recv m]` / `$o->m` / `obj:m()` is recorded as name `m` plus the receiver text | rust.rs:2263-2287 `receiver_text` | §03 of each doc |
| **Qual** | `ns::f()`, `pkg.F()`, `Mod.f`, `m:f` and `Type::m()` are split into a scope receiver and a name, and can bind; `path_receiver` is set where the resolver needs it | Rust `path_receiver` (rust.rs:2041-2086); C++ `split_qualified` (cpp.rs:970-993); Elixir `remote_target` (elixir.rs:98-128) | grep: only cpp, rust and elixir set `path_receiver` to a computed value |
| **Chain** | `a.b().c()`: both calls are recorded, and the outer receiver is typed from the inner callee's declared return type. 🟡 means both calls are recorded but the outer one only fans out by name | Go `go_result_type` for package functions; Zig `zig_generic_call` (libscope.rs:1690-1768) | §03 / §05 of each doc |
| **Peel** | `(x).m()`, casts `(T)x`/`x as T`, constructions `new T().m()`/`T().m()` and `x!`/`await x`/`try x` are peeled, and a cast or construction sets `typed_receiver` | `cgg_lang::receiver::peel` (receiver.rs) | grep: `typed_receiver` is assigned a computed value in cpp, dart, go, groovy, javascript (also used by typescript), php, python, swift, java, csharp, kotlin, ruby and scala |
| **Ctor** | a construction (`new T()`, `T()`, `T.new`, `[T alloc]`, `T::new`) reaches the constructor callable, and constructor delegation (`this(…)`, `super(…)`, `: base(…)`) is a call | `constructor_names` (cross_file.rs:383-391): python, javascript/typescript, php, ruby only; C++ `record_new` (cpp.rs:720-738); Kotlin binds to the class's `Constructor` node | cross_file.rs:383-391 read directly; fixtures `jv`, `cs`, `sw`, `rb`, `sup` |
| **Gen** | explicit type arguments (`f::<T>()`, `f<T>()`, `f[T]()`, `Foo{T}()`) are stripped so the call is recorded under its bare name | cpp.rs:950-968 `callee_name`; csharp.rs:633-646 | §03 of each doc |
| **Super** | `super.m()`, `base.M()`, `parent::m()`, `SUPER::m` and `this->Base::m()` are recorded with a receiver the base-chain walk understands | csharp.rs:573-578 (rewrites `base` to `super`); Perl `SUPER` (perl.rs:422-433) | §03 / §06 of each doc; CLAUDE.md (Java/Scala `super.m()` through the base chain) |
| **?.** | null-safe or optional member calls (`x?.m()`, `x&.m`, `$x?->m()`, `x*.m()`) are recorded as member calls on `x` | csharp.rs:591-599 | §03 of each doc |
| **Ops** | operator, infix and pipe uses that invoke user code (`a + b` to `operator+`/`__add__`/`plus`, `` a `f` b ``, `x \|> f`, `a \| b` in shells) are recorded as calls | Elixir pipes record the right-hand call with arity + 1 (elixir.rs:790-806); Bash and PowerShell pipelines record every command | §03 of each doc; fixture `sup/b.py` (Python `a + b` gives no edge to `__add__`), fixture `kt` (no `Op.plus`) |
| **CallOp** | calling an object or a function-valued variable (`obj()` to `__call__`/`invoke`/`apply`/`operator()`, `$f()`, `f.()`, `(*fp)()`) reaches something | `call_operator_names` (cross_file.rs:394-401): python, php, ruby only; php and ruby never record `local_types`, so only Python works | cross_file.rs:394-401 read directly |
| **Accessor** | property getter and setter syntax (`a.p` for `@property`/`get p()`/C# `P { get; }`/Swift computed properties/Kotlin `get()`) is recorded as a call | Scala paren-less sites (scala.rs:297-324) | fixture `acc` (python, csharp, js, swift, scala, php) |
| **ParenLess** | a call written without parentheses (`helper`, `add 1, 2`, `local_amp;`, `transform 3`) is recorded, with `paren_less` set where pruning is needed | ruby.rs:146-158 and scala.rs:297-324, pruned by `value_refs::prune_paren_less` (value_refs.rs:125-154) | grep: only ruby and scala set `paren_less` |
| **ValRef** | a function passed or stored as a value, *outside* registrar verbs (arguments, assignment right-hand sides, `return`, method references, method groups, captures), becomes a `Via::Reference` site | Python `capture_value_refs` + `capture_value_position` (python.rs:316, 486); Java method references (java.rs:385-441); C# method groups (csharp.rs:456-562); Elixir `&f/n` (elixir.rs:183-207) | grep: `capture_value_refs`/`capture_value_position` are called only from python.rs; `value_ref` is set only in csharp, elixir, java and python; Rust uses the `VALUE_REF_HINT` sentinel (rust.rs:2289-2360) |
| **Registrar** | arguments of a call whose last segment is a registrar verb (`app.get('/x', h)`) are captured as value or string refs. ✅ captured; 🟡 wired but partial or inert; ⬜ a rule for the language declares verbs, or names the missing capture as its gap, but the plugin does not capture; — no registrar-shaped rule exists | `registrar::capture` (registrar.rs:424-505); own implementations in perl.rs:518-611, elixir.rs:413-669 and clojure.rs | grep: `registrar::capture` is called from c, cpp, csharp, elixir, go, java, javascript, php, python and ruby. Registrar verbs are declared for c, clojure, cpp, csharp, elixir, go, javascript, perl, php, python, ruby, rust, swift and typescript (awk over rules.rs `registrars:`). Plugins that ignore `ExtractCtx` (`_ctx`): asm, asyncapi, bash, cmake, dart, erlang, fortran, fsharp, graphql, groovy, haskell, hcl, julia, kivy, kotlin, lean, lua, nix, objc, ocaml, openapi, powershell, proto, r, scala, smithy, solidity, starlark, swift, verilog, vhdl, zig |
| **Arity** | `RefRecord::arity` is set, so overload or clause sets can be narrowed. — for languages with neither overloading nor clause sets of different arity | java.rs:182-184, 367; cpp.rs:820-823; scala.rs:451-457; erlang.rs:255-257; elixir.rs:790-806 | grep: `arity` is assigned a computed value only in cpp, elixir, erlang, java and scala; `OVERLOAD_ARITY_LANGS = ["java","scala","cpp"]` (type_hints.rs:1583) |
| **Kwargs** | named or keyword arguments are recorded in `RefRecord::kwargs`, so `signature_accepts` can narrow fan-out | python.rs:829-839, used by cross_file.rs:755 | grep: only python.rs fills `kwargs` with a non-empty value |
| **ImplRecv** | a receiver supplied by syntax rather than written at the site: Kotlin scope-function lambdas, Perl invocant, Kivy rule context, Groovy closure delegate, Dart cascades, Swift implicit member `.foo()`, Lean leading-dot `.red` | kotlin.rs:110-142, 250-276, 512-522 (`implicit_receiver`); perl.rs:416-421 (`invocant`) | grep: `implicit_receiver` is set only by kotlin; `invocant` only by perl |
| **Embedded** | calls inside string interpolation, macro arguments or token trees, tagged templates, heredocs and quotations are recorded | rust.rs:1491-1660 `refs_from_token_tree` | fixture `emb` (11 languages), plus §03 of each doc |

## The matrix

| language | Bare | Member | Qual | Chain | Peel | Ctor | Gen | Super | ?. | Ops | CallOp | Accessor | ParenLess | ValRef | Registrar | Arity | Kwargs | ImplRecv | Embedded | notes |
| --- | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | --- |
| rust | ✅ | ✅ | ✅ | 🟡 | ✅ | — | ✅ | — | — | ⬜ | ⬜ | — | — | 🟡 | ✅ | — | — | — | ✅ | Operator-trait uses (`a + b`, `x[i]`) and closure-field calls `(w.cb)()` record no site (rust.rs:2062 `_ => return None`); fn values only in argument position |
| python | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | — | ✅ | — | ⬜ | ✅ | ⬜ | — | ✅ | ✅ | — | ✅ | — | ✅ | Operator dunders, `with` (`__enter__`/`__exit__`), subscript-dispatched `h['a']()` and `@property` reads create no edges |
| kivy | ✅ | 🟡 | 🟡 | 🟡 | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | 🟡 | — | `ids.<id>.m()` and `self.parent.m()` lose their owner; paren-less handler refs `on_press: root.by_ref` are not recorded |
| javascript | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 | — | ✅ | ✅ | — | ⬜ | ⬜ | — | ⬜ | ✅ | — | — | — | ✅ | Functions passed as values are captured only under a registrar verb: `[1].map(mapper)`, `setTimeout(cb)` give nothing even with `--reference-edges` |
| typescript | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 | ✅ | ✅ | ✅ | — | ⬜ | ⬜ | — | ⬜ | ✅ | — | — | — | ✅ | Same walker as JS: value refs only under registrar verbs; `super(...)`, `obj['m']()` and `import()` unrecorded |
| go | ✅ | ✅ | ✅ | 🟡 | ✅ | — | ✅ | — | — | — | ⬜ | — | — | ⬜ | ✅ | — | — | — | — | Funcs passed to ordinary calls (`sortSlice(xs, less)`) and func values called through a variable produce no edge |
| java | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 | 🟡 | ✅ | — | — | — | — | — | ✅ | 🟡 | ✅ | — | — | — | `new Box<String>()` / `new Box<>()` never reach the constructor, and `this(...)`/`super(...)` delegation is not recorded |
| kotlin | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ | ✅ | ✅ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | ✅ | No arity and no `::name` reference capture, although Java (same JVM model) has both; operator/infix calls invisible |
| c | ✅ | ✅ | — | 🟡 | — | — | — | — | — | — | ⬜ | — | — | ⬜ | ✅ | — | — | — | 🟡 | Calls through `(*fp)()`, `f()()`, `table[i]()` dropped; functions named in initializers/assignments are not references |
| cpp | ✅ | ✅ | 🟡 | 🟡 | ✅ | 🟡 | ✅ | ✅ | — | ⬜ | ⬜ | — | — | ⬜ | ✅ | ✅ | — | — | ✅ | Operator expressions and `operator()` are never call sites; function values outside registrar verbs and declaration-construction make no edge |
| csharp | ✅ | ✅ | ✅ | 🟡 | ✅ | ⬜ | ✅ | ✅ | ✅ | ⬜ | ⬜ | ⬜ | — | ✅ | 🟡 | ⬜ | ⬜ | — | ✅ | `new T(...)` and `: this()/: base()` produce no constructor edge; no arity although C# overloads like Java |
| bash | 🟡 | — | ⬜ | — | — | — | — | — | — | ✅ | ⬜ | — | — | ⬜ | — | — | — | — | 🟡 | Calls inside a command's arguments (`echo "$(f)"`) and inside `local`/`export` values are never walked (bash.rs:60-69) |
| ruby | ✅ | ✅ | ✅ | 🟡 | ✅ | ⬜ | — | ⬜ | ✅ | ⬜ | ⬜ | 🟡 | ✅ | ⬜ | ✅ | — | ⬜ | — | ✅ | `Foo.new(...)` never reaches `initialize`; bare `super` draws no edge; setters, operators, `&method(:x)` unrecorded |
| swift | ✅ | ✅ | ✅ | 🟡 | 🟡 | ⬜ | ✅ | ✅ | 🟡 | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ✅ | Constructor calls never bind `init` (swift missing from `constructor_names`); `ExtractCtx` ignored, so no registrar or value-ref capture |
| lua | ✅ | ✅ | ✅ | 🟡 | — | — | — | — | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | — | — | — | — | Callbacks (`pcall(run2)`, `table.sort(t, cmp)`) and local aliases are not references; `require('m').g()` not resolved through the module |
| php | ✅ | ✅ | 🟡 | 🟡 | ✅ | ⬜ | — | ✅ | ✅ | — | ⬜ | ⬜ | — | ⬜ | ✅ | — | ⬜ | — | ✅ | `new Foo()` is not a call site; `$fn()`/`$inv()` never reach `__invoke` (no `local_types`); callable strings/arrays only under registrar verbs |
| dart | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 | ✅ | ✅ | ✅ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | — | ⬜ | ⬜ | ✅ | Cascades `..m()`/`?..m()` record nothing; tear-offs and handlers passed as values unrecorded; unnamed ctor binds same-file only |
| scala | ✅ | ✅ | ✅ | 🟡 | ✅ | ⬜ | ✅ | ✅ | — | ⬜ | ⬜ | ✅ | ✅ | ⬜ | — | ✅ | ⬜ | — | ✅ | Infix method calls, `apply` sugar (`Ops(3)`) and functions passed as values are unrecorded; curried calls get a garbage outer name |
| hcl | ✅ | — | 🟡 | — | — | — | — | — | — | — | — | — | — | ⬜ | — | — | — | — | ✅ | Traversals (`var.x`, `aws_s3_bucket.b.id`, `depends_on`, `module.source`) — Terraform's real dependency form — are not references |
| zig | ✅ | ✅ | ✅ | 🟡 | 🟡 | — | ✅ | — | — | — | ⬜ | — | — | ⬜ | — | — | — | — | — | Field-chain receivers untyped; function values, `@call`, `@export` and function-pointer params not references |
| objc | ⬜ | 🟡 | ✅ | 🟡 | ⬜ | ⬜ | — | 🟡 | — | — | ⬜ | ⬜ | — | ⬜ | — | — | — | — | — | C `call_expression`s are never recorded (no arm in objc.rs:59-160): `NSLog`, `dispatch_async`, every C helper and block call are invisible |
| r | ✅ | ✅ | 🟡 | ⬜ | — | — | — | — | — | 🟡 | ⬜ | — | — | ⬜ | ⬜ | — | ⬜ | — | — | Functions passed as values (`lapply(x, f)`, `do.call(f, ...)`) and string-named calls are not captured; `%op%` infix invisible |
| groovy | ✅ | ✅ | ✅ | 🟡 | ✅ | ⬜ | ❔ | ❔ | ✅ | ⬜ | ⬜ | ❔ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | ⬜ | Paren-less command calls (`println "y"`, the whole Gradle DSL) are silently dropped; no method pointers `this.&m` |
| julia | ✅ | ⬜ | ⬜ | ⬜ | — | 🟡 | ✅ | — | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | ⬜ | — | ✅ | Qualified `Mod.f()` and `obj.f()` are recorded as one dotted name and never bind; broadcast `f.(x)`, pipes and infix unrecorded |
| perl | ✅ | ✅ | 🟡 | 🟡 | — | — | — | ✅ | — | ❔ | ⬜ | — | ✅ | 🟡 | ✅ | — | — | ✅ | ⬜ | `Pkg::f()` never binds, even same-file (intra_file.rs:336 skips `::` names), and records a spurious `Pkg` ref |
| elixir | ✅ | ⬜ | ✅ | — | — | — | — | ⬜ | — | ✅ | ⬜ | — | 🟡 | ✅ | ✅ | ✅ | — | — | ✅ | Variable-receiver calls `x.f()` and anonymous `f.()` never bind; special forms and `@attr` lines are emitted as external call noise |
| erlang | ✅ | — | 🟡 | — | — | — | — | — | — | — | ⬜ | — | — | ⬜ | — | ✅ | — | — | ❔ | `fun f/1` references, `?MACRO(...)` invocations and MFA `apply`/`spawn` unrecorded; `Mod:f()` mis-bound to the local `f` at high |
| fortran | 🟡 | ⬜ | — | — | — | — | — | — | — | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | — | — | — | Type-bound calls `s%m()` are silently dropped; array element reads `arr(i)` are recorded as calls |
| clojure | ✅ | ⬜ | ✅ | — | — | ⬜ | — | — | — | ⬜ | ❔ | — | — | ⬜ | ✅ | ⬜ | — | — | 🟡 | Functions passed as values (`(map inner xs)`, `partial`, `comp`) unrecorded; `#(...)` bodies skipped; `#_`/`comment` forms produce false high edges |
| haskell | 🟡 | — | ⬜ | — | — | — | ❔ | — | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | — | — | — | — | Qualified calls `M.lookup` are never emitted — idiomatic `import qualified` code produces no edges; operators, composition, nullary refs dropped |
| ocaml | ✅ | ⬜ | ⬜ | — | — | ⬜ | — | — | — | ⬜ | ❔ | — | — | ⬜ | ⬜ | — | ⬜ | — | — | `\|>`/`@@` pipelines (infix) record nothing for the applied function, and qualified `B.foo` never resolves |
| powershell | 🟡 | ✅ | 🟡 | 🟡 | ⬜ | ⬜ | — | ⬜ | — | ✅ | ⬜ | — | — | ⬜ | — | ⬜ | ⬜ | — | ✅ | Command matching is case-sensitive and aliases are never expanded; `[T]::new()` misses the constructor; cast receivers bind false high |
| solidity | ✅ | ✅ | ✅ | ❔ | ⬜ | ⬜ | — | 🟡 | — | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | ⬜ | — | — | Modifier invocations, base-constructor calls, `emit`/`revert` and `new C(...)` (garbled name `new Other`) make no edge |
| fsharp | ✅ | 🟡 | 🟡 | ⬜ | ⬜ | ⬜ | ✅ | ❔ | — | ⬜ | ❔ | ❔ | — | ⬜ | ⬜ | ⬜ | ⬜ | — | ✅ | Pipelines `x \|> f` (the dominant style) record nothing; receivers of two or more segments are silently dropped |
| starlark | ✅ | ✅ | 🟡 | 🟡 | — | — | — | — | — | — | ⬜ | — | — | ⬜ | — | — | ⬜ | — | — | Top-level calls (every BUILD-file macro invocation) have no enclosing callable, so BUILD files contribute no edges; `rule(implementation=_impl)` not a ref |
| cmake | 🟡 | — | — | — | — | — | — | — | — | — | ⬜ | — | — | ⬜ | — | — | — | — | — | Command names are case-insensitive in CMake but matched case-sensitively; `cmake_language(CALL fn)`/`variable_watch` invisible |
| nix | ✅ | ✅ | ❔ | — | — | — | — | — | — | ⬜ | ⬜ | — | — | ⬜ | — | — | — | — | ✅ | Curried applications are recorded twice at the same byte; parenthesised callees, pipes and functions passed as values dropped |
| verilog | 🟡 | ⬜ | ⬜ | — | — | — | — | ⬜ | — | — | — | — | ⬜ | — | — | — | — | — | ⬜ | Task enables and function calls (`twice(3)`, `show;`) are not recorded — only module/interface instantiation and `$system` calls |
| vhdl | ⬜ | — | ⬜ | — | — | — | — | — | — | ⬜ | — | — | ⬜ | — | — | ⬜ | — | — | — | Only `architecture ... of <entity>` is a reference; function/procedure calls and entity/component instantiation are not (UVVM: 0 edges) |
| asm | 🟡 | — | — | — | — | — | — | — | — | — | ⬜ | — | — | ⬜ | — | — | — | — | ⬜ | `.L` local targets, ARM `b.cond`, RISC-V/AArch64 compare-branches unrecorded; `call *%rax` records a bogus site `rax` |
| smithy | ✅ | — | 🟡 | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Shape ids inside trait bodies (`@references([{resource: City}])`) and elided `$member`s are not references |
| proto | ✅ | — | 🟡 | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Package qualifier stripped (`.a.Meta` -> `Meta`); types in `extend` blocks and `option` values not recorded |
| graphql | ✅ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Directive applications, root operation types in `schema {}`, `extend` blocks and executable documents are not references |
| openapi | ✅ | — | 🟡 | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | Pointer path ignored (responses vs schemas collide); `$ref` under `example:` counted; discriminator `mapping`/`operationRef` not read |
| asyncapi | ✅ | — | ⬜ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | File part of a pointer discarded and binds the local same-named shape at high; deep pointers and `~1` escapes resolve wrongly |
| lean | ✅ | ✅ | ✅ | 🟡 | — | — | ⬜ | — | — | ⬜ | ❔ | — | — | ⬜ | — | — | ⬜ | ⬜ | ✅ | `<\|`/`\|>`/`$` pipes and `@f`explicit calls dropped silently; type annotations (`(xs : List Nat)`) recorded as calls |

### Column totals

| column | ✅ | 🟡 | ⬜ | — | ❔ |
| --- | --: | --: | --: | --: | --: |
| Bare | 37 | 7 | 2 | 0 | 0 |
| Member | 25 | 3 | 6 | 12 | 0 |
| Qual | 20 | 13 | 7 | 5 | 1 |
| Chain | 0 | 24 | 3 | 18 | 1 |
| Peel | 14 | 2 | 4 | 26 | 0 |
| Ctor | 2 | 6 | 12 | 26 | 0 |
| Gen | 12 | 1 | 1 | 30 | 2 |
| Super | 12 | 2 | 4 | 26 | 2 |
| ?. | 8 | 1 | 0 | 37 | 0 |
| Ops | 3 | 1 | 21 | 20 | 1 |
| CallOp | 1 | 0 | 31 | 10 | 4 |
| Accessor | 1 | 1 | 9 | 33 | 2 |
| ParenLess | 3 | 1 | 3 | 39 | 0 |
| ValRef | 4 | 2 | 33 | 7 | 0 |
| Registrar | 12 | 2 | 9 | 23 | 0 |
| Arity | 5 | 0 | 11 | 30 | 0 |
| Kwargs | 1 | 0 | 16 | 29 | 0 |
| ImplRecv | 2 | 1 | 4 | 39 | 0 |
| Embedded | 19 | 3 | 4 | 19 | 1 |

## Parity gaps

Ranked by how many edges or dead-code verdicts the gap costs, weighted by how close a comparable language's implementation is. Effort: **S** = one plugin arm or table entry; **M** = a new pass in one plugin, or the same small change in several plugins; **L** = new resolver or grammar work.

1. **Functions passed as values outside registrar verbs (ValRef).** Python, Java, C# and Elixir capture them, and Rust captures any bare argument. JavaScript, TypeScript, Go, Kotlin, C, C++, Ruby, PHP, Swift, Dart, Scala, Lua, Zig, Groovy, Julia, R, Haskell, OCaml, F#, Clojure, Erlang, Solidity, Starlark, Nix and Lean do not. This is the largest dead-code false-positive class across the corpus.
   - Evidence: `capture_value_refs`/`capture_value_position` are called only from python.rs (python.rs:316, 486). Fixture `js/a.js`: `[1].map(mapper)` and `setTimeout(cb, 1)` give 0 edges with `--reference-edges`. Fixture `go/a.go`: `sortSlice(nil, less)` gives no `less` reference. Fixture `kt/a.kt`: `listOf(1).map(::topHelper)` gives no edge. Rule gap texts name it: rules.rs:5192 (Swift Vapor), 5472/5494 (Dart), 5952 (Haskell scotty), 6042/6059 (OCaml), 6127 (F# Giraffe), 6279 (R Shiny), 6416 (Julia Genie), 7542/7559 (Kotlin), 7965 (C).
   - Effort: **M** per plugin. The shared helpers exist (registrar.rs:517, 563), and Python's scoping filter `retain_scoped_value_refs` (python.rs:861-897) is the model for keeping precision.
2. **Objective-C records no C function calls (Bare).** C records them; ObjC records only message sends.
   - Evidence: objc.rs:59-160 has no `call_expression` arm; objc §03 fixtures f1 and f4. Every C helper, `NSLog`, `dispatch_async` and block invocation in a `.m` file is invisible, and C functions in `.m` files read as dead.
   - Effort: **S**. Reuse the C plugin's `record_call` shape (c.rs:354-383).
3. **Constructions do not reach constructors (Ctor).** This works in Python, JS/TS, Kotlin and C++, and in Java for non-generic types. C#, Swift, Ruby, PHP, Scala, Groovy, Solidity, F#, PowerShell, Objective-C, OCaml and Clojure produce no constructor edge, and Dart binds only within the same file.
   - Evidence: `constructor_names` covers only python, javascript/typescript, php and ruby (cross_file.rs:383-391). PHP and Ruby never emit the bare `Foo(…)` form that table maps: PHP has no `object_creation_expression` arm (php §03), and Ruby's `Foo.new` is a method named `new`. C# `object_creation_expression` is not recorded (fixture `cs/a.cs`: no `Configure -> P.P`). Swift `Engine()` does not bind `Engine.init` (fixture `sw/a.swift`). Ruby `Coll.new(1)` gives no edge to `initialize` (fixture `rb/a.rb`).
   - Effort: **S–M** per language: emit the construction as a call, then add a `constructor_names` entry (Swift `init`, C# the type name, Ruby `new` → `initialize`).
4. **Constructor delegation and `super()` (Ctor/Super).** Java `this(…)`/`super(…)`, JS/TS `super(…)`, C# `: this()`/`: base()`, C++ base and member initializers, Solidity base-constructor calls and Ruby bare `super` are not recorded. Python's `super().__init__()` and Kotlin's base-chain calls do work.
   - Evidence: fixture `jv/Box.java` (no `Box() -> Box(T)` edge); fixture `sup/a.js` (`super()` gives no edge even though `P.constructor` exists); csharp §03 fixture f9; cpp §03 fixture f13; solidity §03 fixture f8; ruby §03 fixture p3.
   - Effort: **S** each: one walker arm per node kind.
5. **Qualified calls that never bind (Qual).** Rust, Go, Elixir, Clojure, Zig and Lean split qualified calls and bind them. These do not:
   - Haskell drops `qualified`-headed applications entirely (haskell §03).
   - OCaml (`B.foo`) and Julia (`Mod.f`) record the dotted text with no split (ocaml §03, julia §03).
   - Perl `Pkg::f()` is skipped by intra_file.rs:336, and no other step splits it.
   - F# drops receivers of two or more segments with no audit record (fsharp §03).
   - C++ leaves `::ns::f()` unresolved (cpp §03 fixture f2 L27).
   - R `pkg:::f` keeps a stray colon (r.rs:186).
   - Effort: **M**. Split into receiver + name with `path_receiver`, as `remote_target` (elixir.rs:98-128) does, then route through a module index.
6. **Arity in overloading languages (Arity).** Java, Scala and C++ (and the clause languages Erlang and Elixir) record arity. Kotlin, C#, Swift, Groovy, Solidity, F#, Clojure (multi-arity `defn`), Julia, PowerShell and Fortran overload but do not.
   - Evidence: grep, where `arity` is assigned only in cpp, elixir, erlang, java and scala; `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583); kotlin §03 ("no overload narrowing"); csharp §03.
   - Effort: **S** per plugin (the `arg_count` helper is in plugins.rs:67), plus adding the language to `OVERLOAD_ARITY_LANGS` after a corpus check.
7. **Pipes and operators (Ops).** Elixir pipes are recorded with arity + 1. F#, OCaml, Haskell, Julia, Lean, Nix and Clojure (threading macros) drop pipe and composition operands. Their docs call this the dominant call style (fsharp §03, ocaml §03 and rules.rs:6042/6059). No language records operator overloads (`operator+`, `__add__`, `plus`, `def +`).
   - Effort: **M** for pipes, mirroring elixir.rs:790-806 for each grammar's infix node; **L** for operator overloads, which need operand types.
8. **Registrar capture not wired, or partly wired (Registrar).** Swift's `aws-lambda` rule declares the verb `run`, but swift.rs:24 ignores `ExtractCtx`, so it can never match. C#'s `registrar.rs` `IDENT_KINDS` (registrar.rs:38-63) lacks `member_access_expression`. Fixture `cs/a.cs`: `app.MapGet("/b", Local)` mints an `aspnet-minimal` entry, while `app.MapGet("/a", Handlers.List)` mints none (only the method-group `ref` edge). Java's capture runs but is inert, because no Java rule declares `registrars`. Kotlin, Dart, Julia, R, Haskell, OCaml and F# have rules whose gap text names the missing capture.
   - Effort: **S** for C# (one `IDENT_KINDS` entry) and for Swift (thread `ctx` through and call `registrar::capture`); **M** for each of the others.
9. **Call operator and function-valued variables (CallOp).** Only Python maps `obj()` to `__call__`. The PHP (`__invoke`) and Ruby (`call`) entries in cross_file.rs:394-401 are unreachable, because neither plugin records `local_types` (php §03, ruby §03). Kotlin `invoke`, Scala `apply`, Dart `call`, C++ `operator()`, C `(*fp)()` and Elixir `f.()` have no handling. C# delegate invocation can bind a **false high** to a same-named method (csharp §03 fixture f11b).
   - Effort: **M**. Add per-language `call_operator_names` entries and record `local_types` for PHP and Ruby.
10. **Verilog and VHDL subprogram calls (Bare).** Verilog records only instantiations and `$system` calls. VHDL records only `architecture … of`, which gives UVVM 1,036 callables and 0 edges (README.md:1350). Task enables, function and procedure calls, and VHDL entity/component instantiation are absent, although the grammars have the nodes (vhdl §03, verilog.rs:6-8).
    - Effort: **M**.
11. **Groovy paren-less command calls (ParenLess).** Ruby, Scala and Perl record paren-less calls. Groovy drops `println "y"` and the whole Gradle and Spock DSL silently: there is no `juxt_function_call` arm, and the `function_call` arm is dead (groovy.rs:124-128).
    - Effort: **M**, because the grammar misparses some statements.
12. **Receiver peeling missing (Peel).** These languages have casts and parenthesised receivers that are not peeled:
    - Objective-C: `[(A *)o ping]` (objc §03 fixture f2).
    - Solidity: `IVault(addr).f()`; the plugin never calls `receiver::peel` (solidity §03).
    - F#: `(a :> Animal).Speak()` records nothing (fsharp §03).
    - PowerShell: `([Base]$this).Hello()` binds itself at **high** (powershell §03 bis2).
    - Effort: **S**. Add `receiver.rs` table entries.
13. **Fortran type-bound calls `s%m()` (Member).** They are silently dropped, with no audit record (fortran.rs:176-202 takes only an `identifier` child).
    - Effort: **S**.
14. **Case-insensitive command names (Bare).** PHP lowercases names in `PhpIndex` (libscope.rs:1916-1923). CMake (cmake.rs:134) and PowerShell (powershell §03 f12) match case-sensitively, although both languages are case-insensitive.
    - Effort: **S**.
15. **Bash command arguments and declarations not walked (Bare/Embedded).** `echo "$(f)"`, `local y=$(f)` and `export Z=$(f)` lose `f` (bash.rs:60-69). Wrapped targets (`sudo f`, `xargs f`, `trap h EXIT`) are lost too.
    - Effort: **S**.
16. **Keyword or named arguments (Kwargs).** Only Python records them. Kotlin, C#, Swift (argument labels decide the overload), Scala, Dart, Ruby, PHP 8, Groovy, Julia, OCaml, F#, Starlark, R, PowerShell, Solidity and Lean do not.
    - Evidence: grep, where only python.rs fills `kwargs`; swift §03 ("argument labels not recorded").
    - Effort: **M**.
17. **Accessor calls (Accessor).** Scala paren-less sites bind `a.p` to `def p`. Python `@property`, JS/TS getters, C# properties, Swift computed properties, Kotlin `get()`, Dart getters and setters, Ruby setters (`self.x = 1` is recorded as `x`, not `x=`), PHP `__get` and Objective-C dot syntax produce no call.
    - Evidence: fixture `acc` (python, csharp, js, swift and php give no edge; scala gives `U::u --> A::p`).
    - Effort: **M**.
18. **Java generic construction (Ctor/Gen).** C#, Scala and Kotlin strip type arguments at construction; Java keeps `Box<String>` as the callee name (java.rs:172-188).
    - Evidence: fixture `jv/Box.java` (raw `new Box()` binds; both generic forms bind nothing).
    - Effort: **S**.
19. **Implicit receivers beyond Kotlin (ImplRecv).** Kotlin types bare calls inside `run`/`apply`/`with`. These do not:
    - Groovy `with {}` and closure delegates (cross_file.rs:4084 comment).
    - Dart cascades `p..run()..shout()`, which record nothing (fixture `dt/a.dart`).
    - Swift implicit members `.shared()`, which are dropped (swift.rs:269-273).
    - Lean `.red`, which is never bound (lean §03).
    - Effort: **M**.
20. **Duplicate sites for curried applications.** Nix (nix.rs:155-163), F# (fsharp.rs:115-118) and Perl (perl.rs:177-178) record the same callee twice at one byte. Edges dedupe, but the audit buckets double-count. Haskell and OCaml record only the innermost application, so they do not duplicate.
    - Effort: **S**.
21. **Calls embedded in strings (Embedded).** They are recorded in most languages that have such syntax (Embedded column). Groovy GStrings (`"${h()}"`, fixture `emb/a.groovy`) and the Perl `@{[ h() ]}` idiom (fixture `emb/a.pl`) give 0 edges. Verilog and asm macro invocations are not expanded.
    - Effort: **S–M**.
22. **Assembly branch coverage.** `.L` targets, ARM `b.cond`/`bl.w` (the `b.eq` entries in asm.rs:71-86 can never match the grammar's `word`), RISC-V `beq`/`tail`, and AArch64 `cbz`/`tbz`/`blr` are not recorded. `call *%rax` records a bogus `rax` (asm.rs:175).
    - Effort: **S**.

## Doc corrections

1. **README.md:1514-1517 overstates function-as-value capture.** It lists "python, javascript, typescript, go, java, csharp, php, ruby, rust, elixir and perl" as covered. For JavaScript, TypeScript, Go, PHP, Ruby and Perl, values are captured **only** as arguments of a registrar verb. Python, Java, C#, Elixir and Rust capture them outside registrar verbs.
   - Evidence: grep (only python.rs calls `capture_value_refs`/`capture_value_position`); fixtures `js/a.js` and `go/a.go`.
   - The JavaScript and Go docs already say so (javascript §03; go §03). This is a README error, not a language-doc error, and the language docs should not be changed for it.
2. **csharp.md §03, "Partial: registrar IDENT_KINDS … (unverified by fixture)": this is now verified.** In fixture `cs/a.cs`, `app.MapGet("/a", Handlers.List)` minted no `aspnet-minimal` entry, while `app.MapGet("/b", Local)` did. `Handlers.List` still produced a `ref` edge under `--reference-edges`, through the C# method-group pass (csharp.rs:456-562), so the only loss is the framework entry.
3. **javascript.md §03: "A bare `super()` in a constructor produced no edge in fixture f5 (the base had no constructor)" is misleading.** `super(…)` is never recorded, even when the base declares `constructor()`. In fixture `sup/a.js`, `K.constructor` has no edge to `P.constructor`. typescript.md §03 gives the correct cause (the callee kind is `super`; javascript.rs:1236 returns for any non-identifier, non-member callee).
4. **Scorecard grades are on inconsistent scales.**
   - rust.md rates 03 `full` while listing no operators, no closure-field calls and no arity. java.md rates `partial` with method references and arity.
   - objc, fortran, haskell, verilog and vhdl use `minimal`, which is not one of the `full/partial/none` values the other docs use.
   - About half the docs write the scorecard key as `03 Call-site forms` without the dot, which breaks a `grep '| 03\.'`. The affected docs are python, javascript, typescript, go, java, cpp, ruby, php, dart, scala, hcl, groovy, julia, perl, elixir, fortran, solidity, cmake, nix, verilog, asm, smithy and openapi.
   - These are not factual errors, but the matrix above regrades them on one scale.

## Side findings (outside section 03, recorded for the owning section)

- **Stale rule gap text.** rules.rs:7559 (Kotlin) says "cgg's Kotlin plugin records no class supertypes". kotlin.rs:174-198 does record class bases (`kotlin_bases`, `record_class`), as CLAUDE.md also states. The value-reference half of the gap text is still accurate (fixture `kt/a.kt`).
- **C# property getter callable name.** In fixture `acc/a.cs`, `public int P { get { … } }` was extracted as the callable `A.get`, not `A.P` or `A.P.get`. Two properties in one class would therefore share a qualified name. This belongs to section 02 and was not investigated further.
- **Swift computed property not extracted.** In fixture `acc/a.swift`, `var p: Int { return 1 }` produced no callable at all; only `u` appeared. This belongs to section 02.
- **CLAUDE.md wording.** CLAUDE.md says "the Java, Scala and C++ plugins set `RefRecord::arity`". The Erlang (erlang.rs:255-257) and Elixir (elixir.rs:790-806) plugins set it too, for clause narrowing. The CLAUDE.md sentence is about overload sets, so it is not wrong, only narrower than what the plugins do.
- **NiceGUI keyword handlers.** Python's registrar matcher does not read keyword arguments (`on_click=save`), per the gap text at rules.rs:8213. This is the one known hole in Python's otherwise ✅ registrar column.

## Fixtures run for this matrix

All fixtures are under `<scratch>/parity/matrix-03/` and were run with `target/release/cgg <dir> [--reference-edges]` (mermaid output).

| fixture | claim checked | result |
| --- | --- | --- |
| `cs/a.cs` | C# registrar capture of `Handlers.List`; `new P(2)` | `ref` edges to `Handlers.List` and `Startup.Local`; `aspnet-minimal` entry only for `/b`; no edge to `P.P` |
| `kt/a.kt` | Kotlin `::topHelper`; `a + a` | only `main --> Op` (constructor); no edge to `topHelper` or `Op.plus` |
| `js/a.js` | JS `[1].map(mapper)`, `setTimeout(cb, 1)` | 0 edges, even with `--reference-edges` |
| `sw/a.swift` | Swift `Engine()` | `use -->\|?\| Engine.go` only; no edge to `Engine.init` |
| `go/a.go` | Go `sortSlice(nil, less)` | `main --> sortSlice` only; no reference to `less` |
| `jv/Box.java` | Java `new Box<String>()`, `new Box<>()`, `new Box()`, `this(null)` | one edge, from the raw `new Box()`; generic forms and `this(...)` bind nothing |
| `dt/a.dart` | Dart cascades | 0 edges |
| `rb/a.rb` | Ruby `Coll.new(1)` → `initialize` | 0 edges |
| `sup/a.js` | JS `super()` with an explicit base constructor | 0 edges |
| `sup/b.py` | Python `a + b` → `__add__` | 0 edges |
| `rs/a.rs` | Rust `(w.cb)()`; `map(conv)` | `run -->\|?ref\| conv` only; no site for `(w.cb)()` |
| `emb/*` | a call inside string interpolation, one file per language | 1 edge each for cs, dart, ex, jl, kt, php, py, rb, swift; 0 for groovy and pl |
| `acc/*` | property or getter access as a call | only scala links (`U::u --> A::p`); python, csharp, js, swift and php give no edge |

Code greps used for the cross-plugin columns are cited in the **Columns** table. Each language row's remaining cells come from that language's §03, where every claim cites a file:line, a test or a fixture; ❔ marks the cells that neither a doc nor this pass confirmed.
