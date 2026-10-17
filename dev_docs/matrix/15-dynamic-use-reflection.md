# 15. Dynamic use & reflection

This category covers code that reaches a callable through a **name held as data** rather than through a call cgg can resolve. Examples are a reflective member lookup by literal name (`getattr(o, "m")`, `send(:m)`, `getMethod("m")`, `MethodByName("m")`), a type looked up by string (`Class.forName`, `Type.GetType`, `const_get`, `NSClassFromString`), a function invoked by name (`call_user_func('f')`, `apply(M, F, A)`, `do.call("f")`, `dlsym(h, "f")`), a module loaded by name (`require("x")`, `import_module("x")`), names written inside attribute strings or unexpanded macro arguments, string `eval`, and "magic" dispatch hooks (`method_missing`, `__getattr__`, `AUTOLOAD`, `__call`, `noSuchMethod`). By design cgg never turns such a name into an edge (`crates/cgg-lang/src/plugins/dynuse.rs:7-12`, `crates/cgg-core/src/facts.rs:150-160`). What it does instead is give the dead-code report **doubt**. A literal in an allowlisted reflective position becomes a `DynUse`. Any same-named callable *of the same language* then gets `name-collides-with-screened-site` evidence: screen `reflection` caps the finding at medium, and screen `module-load` is reported without a cap (`crates/cgg-resolve/src/deadcode/mod.rs:617-637, 898-905`; cap rule `crates/cgg-core/src/deadcode.rs:343-350`). Two neighbouring mechanisms feed the same evidence kind. `FileFacts::name_mentions` produces screens `attribute-string` and `value-position` (`deadcode/mod.rs:189-221`). Framework member-source and signature tables *root* members that are named by an attribute string or invoked by a runtime signature (`crates/cgg-core/src/frameworks/rules.rs:11719-11861`). The capture runs only under `--dead-code` (`ExtractCtx::deadcode_signals`, e.g. `python.rs:122-124`, `go.rs:114-116`, `javascript.rs:79-81`), so it never changes the default graph.

## How the machinery works (shared by every row)

- **Allowlist** (`dynuse.rs:26-56`):

  | key | callees |
  | --- | --- |
  | `python` | `getattr`, `setattr`, `hasattr`, `delattr`, `import_module` |
  | `ruby` | `send`, `public_send`, `method`, `respond_to?`, `instance_variable_get` |
  | `java` | `forName`, `getMethod`, `getDeclaredMethod`, `getField` |
  | `csharp` | `GetMethod`, `GetType`, `CreateInstance`, `GetProperty` |
  | `javascript` | `require`, `importScripts` |
  | `typescript` | `require` |
  | `php` | `call_user_func`, `call_user_func_array`, `method_exists` |
  | `go` | `MethodByName`, `FieldByName` |

- **Callers of `dynuse::extract`**: java.rs:57, javascript.rs:80, ruby.rs:54, php.rs:54, go.rs:115, csharp.rs:83, typescript.rs:86, python.rs:123. No other plugin calls it (grep `dynuse::extract` over `crates/cgg-lang/src`).
- **Matching** (`dynuse.rs:85-137`):
  - The node kind must contain `call`, or be `method_invocation` / `invocation_expression`.
  - The callee is read from field `function`, `name` or `method`, and its last `.`/`:` segment must be in the list.
  - Each argument is unwrapped from `argument` if needed. Its kind must contain `string`, or be `symbol` / `simple_symbol`.
  - Quotes and a leading `:` are stripped. The literal must be identifier-shaped: a letter or `_` first, then only alphanumerics, `_` and `.`, at most 128 bytes (`dynuse.rs:59-67`).
  - The recorded name is the last `.` segment (`dynuse.rs:132`).
  - Consequences:
    - non-literal names are never captured;
    - PHP `\` namespaces fail the shape gate;
    - atom-kinded arguments (Erlang/Elixir `:atom`) would not pass the kind gate;
    - grammars whose call node has none of the three fields (e.g. Kotlin, Swift, Groovy) cannot use the shared walker unchanged.
- **Screens** (`deadcode/mod.rs:622-637`):
  - A `via` in `DYN_MODULE_LOADERS = ["require", "importScripts", "import_module"]` (`facts.rs:165`) is `module-load`. Everything else is `reflection`. `reflection` wins when both hit one name.
  - The key is **(language, simple name)** (`mod.rs:899`). So:
    - the screen is receiver-insensitive (any same-named callable of the language is capped, CHANGELOG.md:755-757);
    - it is **language-scoped**: Java reflection does not screen a Kotlin target (fixture `mixed/`, below).
- **Name mentions** (not dynuse, same evidence kind):
  - Rust records `macro-arg` names (rust.rs:1522, 1843) and serde attribute paths `attr:<key>` (rust.rs:2243).
  - The C and C++ plugins record identifier arguments of unexpanded macros (`c_macro.rs:973`; C++ via `cpp.rs:69`).
  - Screens `attribute-string` / `value-position` cap at medium (`deadcode.rs:343-350`).
- **Rooting by name**:
  - `member_sources_for` roots a member named in an attribute string: Java `junit-sources` `@MethodSource`/`@ArgumentsSource`; C# `nunit-runner` `TestCaseSource`/`ValueSource`, `xunit-data` `MemberData`/`ClassData` (rules.rs:11719-11759).
  - `signature_methods_for` roots Java serialization hooks (`java-serialization`: `readObject`, `writeObject`, `readObjectNoData`, `readResolve`, `writeReplace`, rules.rs:11835-11861).
  - Both tables are keyed by `(rule id, language)`, so they apply to Java and C# only.
- **Blind-spot disclosure** (`deadcode/caps.rs:44-67`):
  - Rust: macro args and attribute strings.
  - Python: "getattr/setattr and other string-dispatched calls".
  - JavaScript and TypeScript: "obj[expr] dynamic property access".
  - Kivy: `ids.foo` lookups and others.
  - No other language names a blind spot.
- **The `dyn_uses` plugin signal is dead metadata.**
  - `PluginSignals::dyn_uses` is copied into `LanguageSignals` (`crates/cgg/src/lib.rs:1934`), but `caps::measure` reads only the other bits (`caps.rs:155-191`).
  - Declaring it or not has no observable effect. JavaScript (javascript.rs:35-43) and Go (go.rs:40-49) capture dyn uses without declaring it.
  - The doc comment for `dyn_uses` in `crates/cgg-lang/src/lib.rs:278-283` is merged into `value_refs`' comment, so the field itself is undocumented.

### Fixtures run for this matrix

All fixtures are under `<scratch>/parity/matrix-15/`, run with `target/release/cgg <dir> --dead-code --no-graph --dead-code-format json`.

| dir | construct | result |
| --- | --- | --- |
| `py/` | `getattr(self,"_by_getattr")`, `importlib.import_module("_by_import")`, `__import__("_by_dunder")` | `_by_getattr` medium `reflection`; `_by_import` high `module-load`; `_by_dunder` high, no screen |
| `js/` | `require("byRequire")`, `importScripts(...)`, `o["byBracket"]()` | first two `module-load`; `byBracket` no screen |
| `ts/` | `require(...)`, `importScripts(...)` | `require` → `module-load`; `importScripts` **no screen** |
| `go/` | `reflect.ValueOf(S{}).MethodByName("byMethodName")` | `byMethodName` `reflection`; sibling `byMethod` none |
| `rb/` | `send(:by_send)`, `__send__(:…)`, `Object.const_get("…")`, `require "…"` | only `by_send` `reflection` |
| `php/` | `call_user_func('by_cuf')`, `call_user_func('App\by_ns')`, `is_callable('…')` | only `by_cuf` `reflection` |
| `java/` | `getMethod("…")`, `getDeclaredField("…")`, `MethodHandles.lookup().findVirtual(…,"…",…)` | only `byGetMethod` medium `reflection`; the other two **high** |
| `cs/` | `GetMethod("…")`, `InvokeMember("…")`, `GetField("…")` | only `ByGetMethod` medium; `ByInvokeMember`/`ByGetField` **high** |
| `kt/` | private members via `Class.forName(...)`, `A::class.java.getMethod("…")` | no screen (medium only from other evidence) |
| `mixed/` | Java `Class.forName("p.K").getMethod("ktReflected")` naming a private Kotlin fun | Kotlin target gets **no** screen (language-scoped key) |
| `groovy/` | `invokeMethod("byInvoke")`, `getClass().getMethod("byGetMethod")` | no screen |
| `rs/` | `#[serde(default = "default_hops")]`, `map(by_macro_arg)` inside `println!` | `attribute-string` and `value-position` screens |
| `c/` | `dlsym(h, "by_dlsym")`, unexpanded `REG(by_macro)` | `by_macro` `value-position`; `by_dlsym` no screen |
| `cpp/` | unexpanded `REG(by_macro)`, `QMetaObject::invokeMethod(nullptr, "by_invoke")` | `by_macro` `value-position`; `by_invoke` no screen |
| `star/` | Starlark `getattr(ctx, "_by_getattr")` | no screen (all Starlark findings already `low`) |
| `junit/` | Java `@MethodSource("cases")` | `cases` rooted (not reported); `notCases` high |
| `kotest/` | the same `@MethodSource("cases")` in Kotlin | `cases` **reported** (medium) — not rooted |

## Legend

- ✅ full: the language's common idioms for this capability are captured.
- 🟡 partial: some idioms are captured, and a common one named in the notes is missing.
- ⬜ none: the concept exists in the language and cgg does nothing with it.
- — n/a: the concept does not exist, or is not meaningful, in the language.

## Columns

| column | meaning |
| --- | --- |
| **member** | reflective member lookup by literal name gives a `reflection` screen (`getattr`, `send`, `getMethod`, `MethodByName`, `method_exists`, `value(forKey:)`, `can`) |
| **type** | type/class lookup by literal string gives a screen (`Class.forName`, `Type.GetType`, `Activator.CreateInstance`, `const_get`, `NSClassFromString`, UVM factory `create("x")`) |
| **call** | invocation of a function by literal name, outside a member lookup (`call_user_func`, `send` without receiver, `apply/3`, `do.call`, `dlsym`, `cmake_language(CALL)`, `abi.encodeWithSignature`) |
| **modload** | a literal module load gives a `module-load` screen (`require("x")`, `import_module`, `importScripts`). Languages whose literal `require`/`import` is a *resolved static import* are graded on dynamic/computed loaders only |
| **attr-str** | a member named inside an attribute/annotation string is rooted or screened (`@MethodSource("x")`, `[MemberData("x")]`, `#[serde(default="x")]`), plus runtime-signature hooks |
| **macro-arg** | an identifier inside an unexpanded macro's arguments gives a `value-position` screen |
| **eval** | string code (`eval`, `exec`, `Invoke-Expression`, `Code.eval_string`) is modelled or at least disclosed |
| **magic** | magic dispatch hooks are modelled (`method_missing`, `__getattr__`, `AUTOLOAD`, `__call`, `methodMissing`, `noSuchMethod`, `@dynamicMemberLookup`, `opDispatch`-style) |
| **disclosed** | the dead-code report names a reflection/dynamic blind spot for the language (`caps.rs:44-67`) |

## The matrix

| language | member | type | call | modload | attr-str | macro-arg | eval | magic | disclosed | notes (most important gap) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | — | — | ⬜ | — | ✅ | ✅ | — | — | ✅ | No reflection, by language design. `libloading::Symbol`/`dlsym` by string is unseen (rust.md §15). Serde attribute paths (rust.rs:2243) and macro-arg names (rust.rs:1522, 1843) are screened (fixture `rs/`) |
| python | ✅ | ⬜ | ⬜ | 🟡 | ⬜ | — | ⬜ | ⬜ | ✅ | `__import__("x")` is not a module-load (fixture `py/`). `globals()[n]()`, `operator.attrgetter/methodcaller`, `pydoc.locate` and `__getattr__` forwarding are absent (dynuse.rs:29). Covers `.ipynb` too (the notebook feeds the Python plugin) |
| kivy | ⬜ | ⬜ | ⬜ | — | — | — | — | — | 🟡 | `dispatch('on_x')`, `Factory.<Name>` and `getattr` in KV expressions are not read (kivy.md §15). The disclosed blind spots cover `ids.foo` but not string dispatch (caps.rs:58-66) |
| javascript | ⬜ | ⬜ | ⬜ | 🟡 | — | — | ⬜ | ⬜ | ✅ | No reflection screen at all. `o["x"]()`, `Reflect.get/apply`, `globalThis["f"]` give nothing (fixture `js/`). Only `require`/`importScripts` literals (module-load, uncapped). Dynamic `import("x")` is absent |
| typescript | ⬜ | ⬜ | ⬜ | 🟡 | — | — | ⬜ | ⬜ | ✅ | As JS, and it also lacks `importScripts`, which JS has (dynuse.rs:49-50, fixture `ts/`). `Reflect`, bracket access and `import()` are not screened |
| go | ✅ | — | ⬜ | ⬜ | — | — | — | — | ⬜ | `plugin.Open`/`Lookup("Sym")` is not screened. `MethodByName`/`FieldByName` work (fixture `go/`). `Method(i)` is index-based and unscreenable |
| java | 🟡 | 🟡 | ⬜ | ⬜ | ✅ | — | ⬜ | ⬜ | ⬜ | `getDeclaredField`, `getConstructor`, `ClassLoader.loadClass` and `MethodHandles.findVirtual/findStatic` stay **high** (fixture `java/`). `@MethodSource` rooting and serialization hooks work (rules.rs:11721, 11837) |
| kotlin | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | — | ⬜ | No dynuse at all, even for Java APIs written in Kotlin (fixture `kt/`). Java-side reflection never screens Kotlin targets (fixture `mixed/`). `@MethodSource` in Kotlin is not rooted (fixture `kotest/`) |
| c | — | — | ⬜ | ⬜ | — | ✅ | — | — | ⬜ | `dlsym`/`GetProcAddress("f")` is unseen (fixture `c/`). `dlopen` is not a module load. Unexpanded macro args are screened (c_macro.rs:973) |
| cpp | ⬜ | ⬜ | ⬜ | ⬜ | — | ✅ | — | — | ⬜ | Qt `QMetaObject::invokeMethod(o,"slot")`, `SLOT("x()")` strings, `dlsym`/`GetProcAddress` and plugin factories are unseen (fixture `cpp/`). Macro args are screened (cpp.rs:69) |
| csharp | 🟡 | ✅ | ⬜ | ⬜ | ✅ | — | — | ⬜ | ⬜ | `GetField`, `InvokeMember`, `GetMember`, `GetEvent`, `Delegate.CreateDelegate` and `Assembly.Load` are missing; the first two stay **high** (fixture `cs/`). `TestCaseSource`/`ValueSource`/`MemberData`/`ClassData` roots exist (rules.rs:11733-11756). `nameof` is a value ref (csharp.md §15). `dynamic`/`DynamicObject` is not modelled |
| bash | — | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | `"$fn" args` and `${!var}` become literal externals. `eval` is dropped by the skip list (bash.rs:184). `source "$f"` with a computed path and `command_not_found_handle` are not modelled |
| ruby | 🟡 | ⬜ | 🟡 | ⬜ | — | — | ⬜ | ⬜ | ⬜ | `require "x"` gives no module-load screen, although `require` is in `DYN_MODULE_LOADERS` (it is missing from Ruby's list, dynuse.rs:31-39; fixture `rb/`). `__send__`, `const_get`/`constantize`, `instance_method`, `define_method` and `method_missing` are absent |
| swift | ⬜ | ⬜ | ⬜ | ⬜ | — | — | — | ⬜ | ⬜ | `perform(NSSelectorFromString("x"))`, `#selector`, `value(forKey:)`, `NSClassFromString`, `@dynamicMemberLookup`/`@dynamicCallable` are all unscreened (swift.md §15 f3) |
| lua | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | `_G["f"]()`, `rawget(_G,"f")` and `M[name]()` give nothing (lua.md §15 p1). A literal `require` is a resolved import, but `dofile`/`loadfile`/`load` are not modelled. `__index`/`__call` are not modelled |
| php | 🟡 | ⬜ | 🟡 | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | `call_user_func('App\\f')` fails the identifier gate on `\` (dynuse.rs:59-67, fixture `php/`). Array callables, `is_callable`, `new $class`, `$o->$m()`, `ReflectionMethod`, `__call`/`__callStatic` are absent. PHPUnit `@dataProvider` naming is not handled: only TestNG mentions `dataProvider` in rules.rs:7461-7473, and this is unverified by fixture |
| dart | ⬜ | ⬜ | ⬜ | ⬜ | — | — | — | ⬜ | ⬜ | `dart:mirrors` `invoke(#name)`, `Function.apply`, `noSuchMethod` and deferred `loadLibrary()` are not modelled (dart.md §15) |
| scala | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | `classOf[X].getMethod("m")`/`Class.forName` are plain external calls (scala.md §15 f6). The `Dynamic` trait (`selectDynamic`/`applyDynamic`) is not modelled |
| hcl | — | — | — | — | — | — | — | — | ⬜ | Not a reflective language. `lookup()`/`try()` are map/stdlib calls (hcl.md §15) |
| zig | ⬜ | — | ⬜ | — | — | — | — | — | ⬜ | `@field(obj,"name")` and `@hasDecl(T,"name")` comptime reflection are unrecognised. `@field(...)` as a callee becomes a bogus external (zig.md §15 p1) |
| objc | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | — | ⬜ | ⬜ | `performSelector:`/`NSSelectorFromString`/`@selector`/`respondsToSelector:`/KVC/`NSClassFromString` are invisible (objc.md §15 f5). `forwardInvocation:`/`resolveInstanceMethod:` are not modelled. objc.rs never calls `c_macro` (grep), so ObjC lacks C's macro-arg screen; ObjC macro handling as a whole is unverified |
| r | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | `do.call("f")`, `match.fun("f")`, `get("f")` and `eval(parse(text=…))` are plain calls (r.md §15 f1). `requireNamespace`/`loadNamespace` are not screened |
| groovy | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜ | `invokeMethod("m")`, `getClass().getMethod("m")` (fixture `groovy/`), `obj."$name"()`, `methodMissing`/`propertyMissing`, `Eval.me` are absent. Groovy lacks even the Java list it shares an API with |
| julia | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | `getfield(M, :f)`, `getproperty`, `invokelatest`, `eval`/`@eval` are plain calls (julia.md §15 p1). Overloaded `getproperty` dispatch is not modelled |
| perl | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | `$obj->can('m')`, `$obj->$m()`, `&{"Pkg::$n"}`, `require $mod` and string `eval` are not screened. `AUTOLOAD` is an ordinary unrooted callable (perl.md §15 f2, f4) |
| elixir | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | `apply(__MODULE__, :f, [])` gives no screen (elixir.md §15 f1). `function_exported?`, `Code.ensure_loaded`, `Module.concat` and `Code.eval_string` are absent. Atom literals would also fail the dynuse kind gate (dynuse.rs:124) |
| erlang | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | `apply(M,F,A)`, `erlang:apply/3`, `spawn(M,F,A)`, `rpc:call` and `code:load_file` are unscreened (erlang.md §15 f1/f6). Variable-module calls are mis-bound rather than treated as dynamic |
| fortran | — | — | — | — | — | — | — | — | ⬜ | No reflection in the language. Procedure pointers belong to section 06; `c_funloc`/`dlsym` interop is rare and unmodelled |
| clojure | ⬜ | — | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | `(resolve 'ns/f)`, `requiring-resolve`, `ns-resolve`, `find-var` and `#'var` do not mark the target (clojure.md §15 p1). `load-file`/`load-string`/`eval` are absent |
| haskell | — | — | — | ⬜ | — | — | ⬜ | — | ⬜ | Little runtime reflection (`Typeable`/`cast` is type-based). `hint`/`plugins` dynamic loading and interpretation are unmodelled (haskell.md §15) |
| ocaml | — | — | ⬜ | ⬜ | — | — | — | — | ⬜ | `Callback.register "name" f`/`Callback.lookup "name"` and `Dynlink.loadfile` are unscreened (ocaml.md §15) |
| powershell | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | — | ⬜ | `& $fn` with `$fn="Name"`, `Get-Command Name`, `$o.$m()` (a literal external `$m`), `Import-Module $path` and `Invoke-Expression` are absent (powershell.md §15 f1/f9) |
| solidity | — | — | ⬜ | — | — | — | — | ⬜ | ⬜ | `abi.encodeWithSignature("f(uint256)")` and `address(x).call/delegatecall` give no screen for `f` (solidity.md §15 f2). `fallback` forwarding is not modelled |
| fsharp | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | — | — | ⬜ | ⬜ | The same .NET APIs C# screens (`GetMethod`, `Type.GetType`, `Activator.CreateInstance`) give nothing in F#. `CreateInstance` through a multi-segment receiver is dropped entirely (fsharp.md §15 f8). The NUnit/xUnit member-source tables are C#-only (rules.rs:11733-11756). The `?` operator is not modelled |
| starlark | ⬜ | — | ⬜ | — | — | — | — | — | ⬜ | `getattr(obj,"name")` is not screened although Python's identical builtin is (fixture `star/`). Impact is small: Starlark findings are already `low`. `native.existing_rule` is not modelled |
| cmake | — | — | ⬜ | ⬜ | — | — | ⬜ | ⬜ | ⬜ | `cmake_language(CALL name)`/`DEFER CALL`, `cmake_language(EVAL CODE …)`, `include(${var})` and `variable_watch(var cmd)` give only the outer builtin call (cmake.md §15 t1) |
| nix | ⬜ | — | — | ⬜ | — | — | — | — | ⬜ | `builtins.getAttr "x" self`, `self.${name}` and a computed `import (./. + "/${x}.nix")`/`scopedImport` are unscreened (nix.md §15 f3) |
| verilog | — | ⬜ | — | — | — | — | — | — | ⬜ | SystemVerilog UVM factory `type_id::create("name")`/`+UVM_TESTNAME` string lookup is not modelled. The language doc calls this n/a, see Doc corrections |
| vhdl | — | — | — | — | — | — | — | — | ⬜ | No reflection beyond attributes. Configuration-driven architecture selection belongs to section 06 |
| asm | — | — | — | — | — | — | — | — | ⬜ | No named reflection. Register-indirect calls and jump tables (`call *%rax`) are section 03/06 gaps, not reflection |
| smithy | — | — | — | — | — | — | — | — | — | Interface language, no runtime |
| proto | — | — | — | — | — | — | — | — | — | Interface language. `google.protobuf.Any` packing is runtime-only |
| graphql | — | — | — | — | — | — | — | — | — | SDL has no reflection. Introspection queries are executable documents, which are not extracted |
| openapi | — | — | ⬜ | — | — | — | — | — | — | `operationId` (Connexion `x-openapi-router-controller`) names a host handler by string and is not followed or screened (openapi.md §15) |
| asyncapi | — | — | — | — | — | — | — | — | — | No runtime. `$ref` is handled statically (section 03) |
| lean | ⬜ | — | ⬜ | — | — | — | ⬜ | — | ⬜ | `evalConst`, `Environment.find?`, quoted names `` `foo `` and `#eval`/`run_cmd` are unrecorded. `macro_rules` quoted bodies do not reach `foo` (lean.md §15 p2) |

Counts across the 46 languages:

- **member**: ✅ 2 (python, go), 🟡 4 (java, csharp, ruby, php).
- **type**: ✅ 1 (csharp), 🟡 1 (java).
- **call**: 🟡 2 (ruby, php).
- **modload**: 🟡 3 (python, javascript, typescript).
- **attr-str**: ✅ 3 (rust, java, csharp).
- **macro-arg**: ✅ 3 (rust, c, cpp).
- **eval**, **magic**: 0 in any language.
- **disclosed**: ✅ 3 (rust, python, js) + ✅ ts, 🟡 kivy.

Only 8 of 46 plugins run `dynuse::extract` at all.

## Parity gaps

Ranked by how directly a comparable language already has the capability, and by how often the construct appears. S is under a day, M is a plugin-specific walker or grammar adaptation, L is a new mechanism.

1. **JVM siblings have no reflection or attribute-string support that Java has: Kotlin, Scala, Groovy.**
   - Evidence: dynuse.rs:41-44 is keyed `"java"` only, and no Kotlin/Scala/Groovy plugin calls `dynuse::extract` (grep). Fixtures `kt/` and `groovy/`: `Class.forName`/`getMethod` written in these languages give no screen.
   - **Cross-language blindness**: the screen key is `(language, simple_name)` (`deadcode/mod.rs:899`). So a Java `getMethod("ktReflected")` does not screen the Kotlin target (fixture `mixed/`).
   - `member_sources_for` is `("junit-sources","java")` only (rules.rs:11721), so Kotlin `@MethodSource("cases")` is reported dead (fixture `kotest/`). The same holds for `java-serialization` hooks (rules.rs:11837).
   - Effort **M**:
     - Kotlin/Groovy call nodes do not expose `function`/`name`/`method` fields, so the shared walker (dynuse.rs:99-104) needs a per-grammar callee reader.
     - The member-source and signature tables need `kotlin`/`scala`/`groovy` arms (S each).
     - Keying the screen by JVM family instead of language is S.
2. **F# lacks the .NET reflection list C# has.**
   - Evidence: dynuse.rs:45-48 is `"csharp"` only. fsharp.md §15 f8: `GetMethod` is a plain external call and `Activator.CreateInstance` through a multi-segment receiver is dropped.
   - NUnit/xUnit member-source rooting is C#-only (rules.rs:11733-11756).
   - C# reflection does not screen F# targets (same language-scoped key, mod.rs:899; inferred from code, no F# fixture run).
   - Effort **M**.
3. **JavaScript/TypeScript have no `reflection` screen, unlike Python, which screens `getattr`.**
   - `o["name"]()`, `Reflect.get(o,"name")`, `globalThis["f"]` and `this[k]` with a literal `k` give nothing (fixture `js/`, typescript.md §15 f11), although caps.rs:57-58 discloses the blind spot.
   - TS also lacks `importScripts`, which JS has (dynuse.rs:49-50; fixture `ts/`: `byImportScripts` unscreened).
   - Neither has dynamic `import("x")`.
   - Effort:
     - **S** for `importScripts`/`import` in TS.
     - **M** for literal bracket calls: `subscript_expression` with a string index must be recognised in the callee position rather than as an argument, which the shared walker does not do.
4. **Java and C# API breadth leaves common reflective calls at `high`.**
   - Java: `getDeclaredField`, `getConstructor`/`getDeclaredConstructor`, `ClassLoader.loadClass` and `MethodHandles.lookup().findVirtual/findStatic/findSpecial` are missing (fixture `java/`: `byGetDeclaredField` and `byFindVirtual` stayed **high**).
   - C#: `GetField`, `InvokeMember`, `GetMember`, `GetEvent` and `Delegate.CreateDelegate` are missing (fixture `cs/`: `ByInvokeMember` and `ByGetField` stayed **high**).
   - These are false-confidence findings in the top band.
   - Effort **S**: add names to dynuse.rs:41-48. `findVirtual`'s name is the second argument, which the walker already handles because it scans every argument (dynuse.rs:113).
5. **C/C++/ObjC symbol lookup by string.**
   - `dlsym(h,"f")` and `GetProcAddress(h,"f")` in C/C++, Qt `QMetaObject::invokeMethod(o,"slot")`, and ObjC `performSelector:`/`NSSelectorFromString`/`respondsToSelector:`/`NSClassFromString` are not screened (fixtures `c/`, `cpp/`; objc.md §15 f5).
   - Python, Go, Java, C# and PHP all have a lookup-by-literal screen.
   - Effort:
     - **S** for C/C++: `call_expression` has `function`/`arguments` fields and `string_literal` passes the kind gate, so it needs only a REFLECTIVE row plus a call in c.rs/cpp.rs.
     - **M** for ObjC `message_expression` selectors and `@selector`.
6. **Ruby misses `require` as a module load, plus common reflection.**
   - Ruby `require` is in `DYN_MODULE_LOADERS` (facts.rs:165), but it is absent from Ruby's list (dynuse.rs:31-39).
   - `__send__`, `const_get`/`constantize`, `instance_method`, `public_method` and `define_method` are absent (fixture `rb/`).
   - Python's `import_module` is screened; Ruby's equivalents are not.
   - Effort **S**.
7. **Python misses its own module and lookup forms.**
   - `__import__("x")` is not a module-load (fixture `py/`: `_by_dunder` high).
   - `operator.attrgetter("m")`/`methodcaller("m")`, `pydoc.locate`/`importlib.util.find_spec`, and `globals()["f"]()`/`vars(o)["m"]` (the subscript form, M) are missing.
   - Effort **S** for the named callees.
8. **PHP: namespaced strings, array callables and class lookups.**
   - `call_user_func('App\\f')` fails `is_identifier_shaped` because `\` is rejected (dynuse.rs:59-67; fixture `php/`: `by_ns` unscreened).
   - The `[Cls::class,'m']` array-callable form is ignored, because only string-kinded arguments are read (dynuse.rs:124).
   - `is_callable`, `function_exists`, `class_exists`, `property_exists` and `ReflectionMethod`/`ReflectionClass` are absent.
   - Effort **S** (allow `\` and take the last segment, as `.` is handled at dynuse.rs:132) to **M** (array callables).
9. **Erlang/Elixir MFA dispatch: `apply(M,F,A)`, `spawn(M,F,A)`, `erlang:apply/3`, `rpc:call`, `function_exported?`.**
   - These are the dominant dynamic form in BEAM code (erlang.md §15 f1/f6, elixir.md §15 f1). They are comparable to PHP `call_user_func`.
   - Atom arguments fail the kind gate (`k.contains("string") || symbol || simple_symbol`, dynuse.rs:124). The callee is a qualified `m:f` / `Mod.f` form, and the target is MF-scoped.
   - Effort **M**.
10. **Starlark lacks Python's `getattr` screen.** It has the same builtin with the same semantics (fixture `star/`). Effort **S**: a REFLECTIVE row `("starlark", ["getattr","hasattr"])` plus a call in starlark.rs. The practical cap effect is nil, because Starlark findings are already `low`, but it would add evidence.
11. **Go `plugin.Open`/`Lookup("Sym")`.** These are the Go analogue of `dlsym`/`import_module` (go.md §15). Effort **S**: `Lookup` is a `call_expression` with a string argument. `Open` should be a module-load, which needs `DYN_MODULE_LOADERS` to grow (facts.rs:165).
12. **Dynamic-language "call by name" idioms with no screen at all.**
    - R `do.call("f")`/`match.fun("f")`/`get("f")` (S: plain calls with string args).
    - Lua `_G["f"]`/`rawget(_G,"f")` (M: subscript).
    - Perl `->can('m')` (S), `$obj->$m` (M).
    - Clojure `resolve`/`requiring-resolve`/`ns-resolve` with a quoted symbol (M: the symbol kind).
    - PowerShell `& "Name"`/`Get-Command Name` (M).
    - Julia `getfield(M,:f)`/`invokelatest` (M).
    - CMake `cmake_language(CALL f)` (S).
    - Nix `builtins.getAttr "x"` (S).
    - Solidity `abi.encodeWithSignature("f(...)")` (S, with a parse of the name before `(`).
    - Dart mirrors / `Function.apply` (M).
    - Swift `NSSelectorFromString`/`value(forKey:)` (M).
    - Lean `evalConst` (M).
    - Each is the language's equivalent of PHP `call_user_func` or Python `getattr`. Evidence: the corresponding language doc's §15 fixtures.
13. **Blind-spot disclosure exists for 5 languages only** (caps.rs:44-67).
    - The Ruby, PHP, Java, C#, Go, Perl, Lua, Groovy, Kotlin, Elixir/Erlang, PowerShell and R reports never tell the reader that `send`-with-variable, `$o->$m()`, `apply/3` and similar are invisible. Python's and JS's do.
    - Effort **S** (table entries only). This is the cheapest honesty win.
14. **No language models string `eval` or magic dispatch hooks.**
    - Hooks: `method_missing`, `__getattr__`/`__getattribute__`, `AUTOLOAD`, `__call`/`__callStatic`, `methodMissing`/`propertyMissing`, `noSuchMethod`, `@dynamicMemberLookup`, `selectDynamic`, `forwardInvocation:`.
    - Every language doc reports these as absent. Perl's `AUTOLOAD` and Groovy's `methodMissing` are even extracted as ordinary unrooted callables and reported dead (perl.md §15 f2, groovy.md §15 f1).
    - Effort:
      - **S** to root these hook names as `node:false` runtime-invoked methods per language, as Java serialization hooks already are (rules.rs:11835).
      - **L** to model what they dispatch to.
15. **The screen is receiver-insensitive and non-literal names are uncaptured, in every covered language.**
    - Receiver-insensitive: a single `getattr(x,"save")` caps every `save` in the language (CHANGELOG.md:755-757; `deadcode/mod.rs:899`).
    - Non-literal: `getattr(o, name)`, `send(m)`, `"#{x}_handler"` and f-strings (dynuse.rs:59-67, 124-130).
    - This is a design limit rather than a parity gap, but it is uniform. Effort **L**: it needs receiver typing at the dyn-use site.
16. **The `dyn_uses` plugin signal is never consumed.**
    - JS and Go under-declare it (javascript.rs:35-43, go.rs:40-49), and `caps::measure` ignores it for everyone (caps.rs:155-191). So the capability table cannot tell "the language has no reflection capture" from "nothing was found".
    - Effort **S**: either surface it as a `reflection` column in the capability table or remove the field.

## Doc corrections

- **The covered-language list is wrong in many docs and in CLAUDE.md.**
  - CLAUDE.md (step 7, "`dynuse` covers Python, JS, Java, C#, Ruby and PHP") and the language docs that quote it omit **TypeScript and Go**: kivy, bash, swift, hcl, zig, objc, r, ocaml, fsharp, starlark, cmake, nix, verilog, vhdl, graphql, openapi, asm and lean `.md` §15.
  - Both are in REFLECTIVE (dynuse.rs:50, 55) and call `extract` (typescript.rs:86, go.rs:115). Fixtures `ts/` and `go/` confirm it.
  - cpp.md §15 ("lists python, ruby, java, csharp, javascript, typescript and php", dynuse.rs:24-58) and groovy.md §15 (dynuse.rs:25-50) omit Go.
  - erlang.md §15 ("covers Python, Ruby, JS, Java, C# and PHP") omits TypeScript and Go.
- **kotlin.md §15** marks "`Class.forName("…")` and `::class.java.getMethod("…")` written in Kotlin are not screened" as *unverified by fixture*. It is now **verified** by fixture `kt/`. In addition, Java-side reflection does not screen Kotlin targets (fixture `mixed/`), and Kotlin `@MethodSource` is not rooted (fixture `kotest/`). The doc does not mention either.
- **go.md §15 / §12 and javascript.md §15** say the `signals()` manifest under-declares `dyn_uses`. That is literally true, but it has **no effect**: `LanguageSignals::dyn_uses` is not read by `caps::measure` (caps.rs:155-191), and Python/Java/etc. declaring it changes nothing either. The docs should not present it as a functional defect.
- **python.md §15** cites `is_identifier_shaped` at `dynuse.rs:47-57`. It is at **dynuse.rs:59-67**. (The `REFLECTIVE` row at `:26-29` and the `python.rs:122-124` gate are correct.)
- **javascript.md §15** cites `dynuse.rs:117-125` for "`require(name)` with a non-literal argument is ignored". The literal-kind filter is at **dynuse.rs:123-126**, and the identifier gate is at 127-130.
- **verilog.md §15 / scorecard** grades the category "n/a". SystemVerilog/UVM does have reflection by string: factory `create("name")`, `+UVM_TESTNAME`, `uvm_config_db` string keys. So the grade should be **none** (⬜), not n/a. VHDL's "n/a" stands.
- **ruby.md §15** is accurate. One addition: ruby.md's "No `BLIND_SPOTS` entry for Ruby" applies equally to Java, C#, PHP and Go (caps.rs:44-67). Their docs do not say so.
- **csharp.md §15** is accurate. One detail it omits: because the recorded name is the literal's last `.` segment (dynuse.rs:132), `Type.GetType("Ns.Foo")` and `Activator.CreateInstance("asm","Ns.Foo")` screen callables named `Foo` (the class/constructor), not a member.
- **Code comment, not a language doc**: in `crates/cgg-lang/src/lib.rs:278-283` the doc comment for `dyn_uses` ("Captures identifier-shaped literals…") sits above `value_refs` and is merged with its comment, so `dyn_uses` itself has none.
