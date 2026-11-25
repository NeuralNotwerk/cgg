# solidity

> Revised 2026-11-25 against cgg 0.13.3: `new C(…)` and `constructor() Base(x)` now call constructors, bodiless interface/abstract functions are tagged `abstract`, Solidity has a locals table (a function-typed parameter shadows), an `import` binds the copy in the file it names, and `*.t.sol` is a test file. `file:line` citations not otherwise revised date from the 0.11.0 audit and may have drifted.

The Solidity plugin is `SolidityPlugin` in `crates/cgg-lang/src/plugins/solidity.rs` (399 lines, inline tests included). It parses `.sol` files with the crates.io grammar `tree-sitter-solidity` (workspace pin `"1.2"` at `Cargo.toml:115`; the lockfile resolves 1.2.13, `tree_sitter_solidity::LANGUAGE` at solidity.rs:28-30). Nothing is vendored. The plugin (509 lines in 0.13.3) is a single recursive `SolidityWalker` that does four things:

- pushes a scope for each `contract_declaration`, `library_declaration` and `interface_declaration`;
- records each `function_definition`, `modifier_definition` and `constructor_definition` as a definition, and tags functions inside a `library` with the `library-member` attribute (`LIBRARY_MEMBER`, solidity.rs:9);
- records `import_directive` paths (kind `import`) and `using L for T;` directives (kind `using-for`);
- records `call_expression` sites whose callee is an identifier or a `member_expression`, plus `new C(…)` and base-constructor invocations in a constructor header as constructor calls (since 0.11.1, WS21).

Its locals are collected outside the plugin, by `locals::collect_solidity` (`crates/cgg-lang/src/locals.rs:2671`; see §07).

The plugin declares one signal only, `visibility: true` (solidity.rs:21-26). It does not override `resolver_kind`, `shebangs`, `ts_language_for` or `max_line_bytes`, and it ignores the `ExtractCtx` it is handed (`_ctx`, solidity.rs:34). As a result, registrar verbs, reference sites and dead-code signal switches have no effect on Solidity.

Solidity-specific resolution lives in five places:

- `cgg-resolve/src/heads.rs`: `using_for` (368-370, 617-624) and `libraries_for` (638-648).
- `cgg-resolve/src/cross_file.rs`:
  - file-relative imports (2029-2060);
  - the library-member bare-name filter (2659-2680);
  - the own-scope rule for bare calls (5681-5711);
  - step 4e, `using … for` (6811-6840).
- `cgg-resolve/src/intra_file.rs`: `binds_same_name_set` (827-850).
- `cgg-resolve/src/frameworks/mod.rs`: `match_visibility` (1639-1685).
- `cgg-core/src/frameworks/rules.rs`: the `solidity-public` rule (1416-1438) and `visibility_entries_for` (11677-11691).

Fixtures are under `<scratch>/parity/solidity/` (f1 … f9). Each was run with `target/release/cgg` 0.11.0 as `-t json -o out.json`, and dumped with `show.py` in that directory. Runs that needed exit nodes added `--include-external`.

## 01. Detection & parsing

- **Extension `.sol` only** (solidity.rs:18-20). Fixture f1 audit: `"detected_via": "extension:.sol"`.
- **Shebangs**: none. The plugin uses the trait default of `&[]` (cgg-lang/src/lib.rs:298-300).
- **Content detection**: none. `detect.rs` has no Solidity rule; a grep for `solidity` under `crates/cgg-lang/src` finds only `plugins.rs:48,156` and the plugin itself.
- **Grammar**: one grammar, `tree-sitter-solidity` 1.2.13, with no dialect switch. Its `node-types.json` includes Yul nodes (`assembly_statement`, `yul_function_definition`, `yul_function_call`), but the walker ignores them (§02, §03).
- **Re-parse/fallback**: none. Unlike C#'s `#if` blanking, a file with syntax errors is extracted from whatever tree the grammar produced, with no special recovery.
- **Depth skip**: shared. A tree deeper than `MAX_TREE_DEPTH` (4,000, cgg/src/lib.rs:155) is skipped as `SkipReason::TooDeep` (lib.rs:451). This was not exercised for Solidity (unverified for this language; the generic test is `crates/cgg/tests/deep_nesting.rs`).
- **Minified handling**: there is no `max_line_bytes` override; the shared 2,000-byte-line rule skips locals collection. `--skip-minified` is generic (`crates/cgg/tests/skip_minified.rs`) and was not exercised for `.sol` (unverified).
- **Not supported**:
  - Foundry/Hardhat project files (`foundry.toml`, `remappings.txt`, `hardhat.config.*`) are never read (no Solidity code in `plugins/manifest.rs`; grep).
  - `.yul` standalone files are not claimed by any plugin.

## 02. Callables extracted

Qualified names are `Scope.name`, where the scope is the enclosing contract, library or interface name (`qn`, solidity.rs:66-72). A file-level free function has a bare QN with no file or module prefix, so two files declaring `function toStr` produce identical QNs (fixture f6: `bfn` in `B.sol` and `Z.sol`).

Supported:

- **Contract function** `contract Token { function transfer(address to) public returns (bool) {…} }` → `Token.transfer`, `DefVariant::InherentMethod` (kind `method`) (solidity.rs:106-110, 194-200). Test `contract_methods_and_ctor` (solidity.rs:323). Fixture f1.
- **Constructor** `constructor(uint t) {…}` → `Token.constructor`, `DefVariant::Constructor` (solidity.rs:111-115, 184-186). Test `contract_methods_and_ctor`. Fixture f1 `Token.constructor constructor`.
- **Modifier** `modifier onlyOwner() { _check(); _; }` → `Token.onlyOwner`, `InherentMethod`, with empty visibility (modifiers have no visibility keyword, so `Vis::Unknown`) (solidity.rs:106). Fixture f1 and f2 (`Ownable.onlyOwner`).
- **Library function** `library Math { function add(…) internal pure … }` → `Math.add`, `InherentMethod`, attribute `library-member` (solidity.rs:97, 232-236). Tests `library_function_is_free` (solidity.rs:341; despite its name it asserts the QN `M.add`) and `library_functions_are_marked` (solidity.rs:383). Fixture f1 `Math.add ['library-member']`.
- **Interface member** `interface IToken { function transfer(address, uint) external returns (bool); }` → `IToken.transfer`, `InherentMethod`, bodiless, tagged `abstract` since 0.11.1 (WS22; `attrs::mark_abstract_from`, solidity.rs:111; test `bodiless_functions_are_tagged_abstract`). The "drop abstract when a concrete candidate survives" narrowing now applies: fixture (2026-11-25, `lang-docs/solidity/b`) `IVault public other; other.peek()` binds only `Vault.peek` (medium); in 0.11.0 (f2) it bound both.
- **Abstract/virtual declaration** `function hook() internal virtual;` in an `abstract contract` → `Base.hook` node, bodiless, tagged `abstract` (same test). Fixture f1.
- **Free (file-level) function** `function freeFn(uint a) pure returns (uint) {…}` → `freeFn`, `DefVariant::FreeFunction` (solidity.rs:196-197). Fixture f1 and f8 (`addF`).
- **Overloads**: two `function over(…)` in one contract produce two nodes with the same QN `Token.over`. Fixture f1.
- **Visibility**: the native `visibility` child is kept as a string and normalised: `public`/`external` → `Vis::Public`, `internal` → `Vis::Internal`, `private` → `Vis::Private`, otherwise `Unknown` (solidity.rs:206-220).
- **Signature hint**: `extract_signature` over the definition text (solidity.rs:229). This is what parameter typing reads (§05).

Not extracted (fixtures f1, f8):

- **`fallback()` / `receive()`** (`fallback_receive_definition`): no node. These are chain-invoked entry points (fixture f1, f2: absent from NODES).
- **Events, custom errors, structs, enums and user-defined value types** (`event_definition`, `error_declaration`, `struct_declaration`, `user_defined_type_definition`): no node (fixture f1: `Sent`, `Bad` and `S` are absent).
- **Public state-variable getters** `uint public counter;`: no getter callable. `this.counter()` becomes an exit node `<external>::this::counter` (fixture f8).
- **Yul functions** inside `assembly { function inner(x) -> y {…} }`: not extracted (fixture f8: no `inner` node).
- **Function-typed state variables or parameters**: not callables (expected).
- No `macro:`/generated tagging and no `ClassDecl` records (the plugin writes no `facts.classes`; solidity.rs has no such push).

## 03. Call-site forms

Every site is a `RefRecord` with `name` plus `receiver_hint`; no other `RefRecord` fields are set (solidity.rs:291-297). The callee is `call_expression` → (`expression` →) `identifier` or `member_expression`. For a member call the property is the last identifier/expression child and the object is the first one's text (solidity.rs:241-287).

Supported:

- **Bare call** `_check();` → `name="_check"` with empty receiver. Test `call_captured` (solidity.rs:364). Fixture f1: `Token.onlyOwner -> Token._check high`.
- **Member call** `h.k();` → `name="k", receiver="h"`. Test `call_captured`.
- **Library call** `Math.mul(x, 2)` → `receiver="Math"`, and it binds to `Math.mul` at `high` (fixture f1, line 32).
- **Nested path** `Lb.Math.mul(1,2)` → receiver `Lb.Math`. It still binds `high` to `Math.mul` (fixture f1, line 39).
- **`this.f()`** → `this.peek()` binds `high`, intra-file (fixture f1, line 37).
- **`super.f()`** → `receiver="super"`. It binds by name, at medium, through `cross-file:imports` (fixture f1 line 35, fixture f2 line 19). There is no Solidity-specific super path (§06).
- **`try` external call** `try v.peek() returns (uint) {} catch {}` → recorded like any call; binds `high` to `IVault.peek` (fixture f2, line 27).
- **Chained/cast receivers** `Other(address(0)).ping()`, `IVault(address(this)).deposit(2)` → the receiver hint is the raw object text (`"Other(address(0))"`), which matches no type. The site is unresolved `candidates-in-other-files`, or becomes an exit node with `--include-external` (fixtures f1, f2). The plugin does not call `cgg_lang::receiver::peel`, so the cast is not read as a type.
- **Conversion/cast calls** `IVault(x)`, `Other(x)`, `address(v)` → the type name is recorded as a bare call. It has no callable, so it ends as unresolved `no-candidate-in-file` (fixture f1 `Other`, f2 `IVault`). `address(...)` itself is not recorded: the grammar gives it a primitive-type callee (fixture f2: no `address` site).
- **Struct construction** `S(1)` → bare call `S`, which becomes an exit node `<external>::S` (fixture f1).
- **Function-typed values called** `f(3)` and `fn()` → bare calls with no callee, so exit nodes `<external>::f` and `<external>::fn` (fixture f2).
- **Arity**: not recorded (`RefRecord::arity` is never set). Solidity is also not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:2357).

Partially supported or garbled:

- **`new Contract(...)`** → since 0.11.1 (WS21) a call of `Contract`'s constructor, looked up on the type (`record_call`, solidity.rs:299-320; test `new_and_base_constructor_invocations_call_constructors`). Fixture (2026-11-25, `lang-docs/solidity/a`): `Other o = new Other();` binds `D.loc -> Other.constructor` at `high`. In 0.11.0 the name was the literal `"new Other"` (fixture f1: exit node `<external>::new Other`). `new uint[](n)` names no contract and records nothing.
- **A receiver that is a binary expression** `Fixed.unwrap(a) + Fixed.unwrap(b)` produced an exit node `<external>::Fixed.unwrap(a) + Fixed::unwrap` (fixture f8). The receiver text is wrong for one of the two sites.

Not recorded at all:

- **Modifier invocations** on a function header, `function deposit(uint a) external onlyOwner` and `onlyRole(3)`. These are `modifier_invocation` nodes, which are not walked as calls (only a constructor-header one naming a base is, above). Fixture f2: `Ownable.onlyOwner` has no incoming edge. Fixture f8: `Base.onlyRole` has none. Re-checked 2026-11-25 (`lang-docs/solidity/a`: `function g() public onlyOwner` gives no edge).
- **Base-constructor arguments in the inheritance list** `contract K is Base(1)`. Fixture (2026-11-25, `lang-docs/solidity/a`): no edge to `Base.constructor`. The constructor-header form `constructor() Base(2) {}` is a call since 0.11.1 (WS21, `record_base_constructor_calls`, solidity.rs:266): a header modifier invocation naming one of the contract's bases binds `D.constructor -> Base.constructor` at `high` (same fixture).
- **`emit Event(...)`** (`emit_statement`) and **`revert Error(...)`** (`revert_statement`). No site at all (fixture f1: neither `Sent` nor `Bad` appears in unresolved or external).
- **Yul calls** `inner(2)` and `add(x, 1)` inside `assembly`. Not recorded (fixture f8).
- **User-defined operators** `a + b` via `using {addF as +} for Fixed global` (fixture f8: no site).
- **Function references as values** `cb(helper)`, `this.deposit` and `this.peek.selector`. There is no `value_ref` support (the `value_refs` signal is off, solidity.rs:21-26). Fixture f2: `Vault.helper` has no incoming edge.
- **Registrar arguments**: no registrar capture (`plugins/registrar.rs` is never called; grep).
- **Paren-less, explicit-generic and call-operator forms**: not applicable to the language.

## 04. Imports & module resolution

Supported:

- **`import "./X.sol";`** → `ImportRecord { kind: "import", path: "./X.sol", alias: "" }` (solidity.rs:116-133). Test `import_captured` (solidity.rs:353).
  - Resolved file-relative, together with Dart and Nix (cross_file.rs:2029-2048). Each of `path`, `path.sol`, `path.dart` and `path.nix` is tried through `collect_include_defs` (cross_file.rs:3385+), which matches the exact normalised path first and then a path suffix.
  - Every definition of the imported file then enters `direct_imports`. Fixture f6: `afn()` in `C.sol`, which imports `A.sol`, binds `high`.
- **`import {A as B} from "./X.sol";`, `import * as L from "./X.sol";` and `import "./X.sol" as L;`** → only the path string is captured. The import list and the alias are dropped (`alias: String::new()`, solidity.rs:126). This has two consequences:
  - A named import still brings **every** definition of the file into scope; the symbol list is not used to narrow.
  - The alias half of cross_file.rs:2049-2059 (`module_aliases`) never fires for Solidity. Fixture f1: `H.go()` is unresolved `candidates-in-other-files`. Fixture f4: `S2.toStr(2)` is unresolved.
- **Suffix fallback**: `import "contracts/utils/Strings.sol"` reaches `lib/openzeppelin-contracts/contracts/utils/Strings.sol` by path suffix (fixture f4, `U2.b -> toStr high`).
- **`using L for T;` / `using L for *;`** → `ImportRecord { kind: "using-for", path: "L", alias: "T" | "*" }` (solidity.rs:143-165). `FileHeads::using_for` reads it in declaration order (heads.rs:617-624), and `libraries_for` serves it (heads.rs:638-648; it strips `[]` and the text after a space from the target). It drives step 4e (§05).

Partial:

- **`using {f, g as +} for T global;`** (brace form, Solidity 0.8.13+): the plugin looks only for a direct `type_alias` child (solidity.rs:144), so the brace list is not recorded. Fixture f8: `a.addF(b)` on `Fixed a` is unresolved.
- **Transitive imports**: Solidity makes the symbols a file imports visible to its importers. `collect_include_defs` recurses only into imports of kind `include`/`system-include` (cross_file.rs:3510-3522), so a Solidity `import` is not followed. Fixture f6: `bfn()`, reachable through `C → A → B`, bound at **medium** to both `B.sol`'s `bfn` and an unrelated, unimported `Z.sol` `bfn`. That is a by-name match, not import-following; the exact step that produced it is unverified.

Not supported:

- **Import remappings**: `@openzeppelin/…`, `forge-std/…` and similar, through `remappings.txt`, `foundry.toml` or `node_modules`. No manifest is read. Fixture f5: `import "@openzeppelin/contracts/utils/Strings.sol"` with the file at `lib/openzeppelin-contracts/contracts/utils/Strings.sol` leaves `toStr()` unresolved `candidates-in-other-files`.
- **External-module refusal (`heads.rs` `external_head`)**: Solidity is not in the language list (heads.rs:657-669). An unresolvable import is never classified `external-module`, and nothing marks a call through it as external rather than missing.
- **`FileFacts::packages`, `ModuleIndex` / `not-in-imported-module`, `libscope.rs`**: none for Solidity (grep of `modules.rs` and `libscope.rs` finds no `solidity`).
- **Include-style binding (WS19, 0.11.1).** A call now binds the copy in the file the `import` names (`included_copies`, `crates/cgg-resolve/src/cross_file.rs:8485`), never whichever same-named copy was indexed last; an import path several files share expands to each at `medium`. Fixture (2026-11-25, `lang-docs/solidity/a`): `import "./x/Lib.sol"` with `libfn` in both `x/Lib.sol` and `y/Lib.sol` binds `x/Lib.sol`'s at `high`.
- **Import gating does work negatively**: a file with no imports does not reach other files' functions by name. Fixture f3 `E.sol`: `freeA()` and `only()` are unresolved `candidates-in-other-files`.

## 05. Type inference & receiver typing

Supported:

- **Parameter types** `function useParam(IVault v, Pos memory p)` → the generic type-first parameter parser (`collect_param_types`, type_hints.rs:1413-1488 → `parse_type_first_param`, 2166-2210). It handles the data-location word explicitly: "Words after the type that are not the name (Solidity `Foo memory x`)" keep the first word (type_hints.rs:2189-2191). Only a type beginning with an uppercase letter is kept (type_hints.rs:2199).
  - Fixture f2: `v.deposit(1)` binds `high` to `IVault.deposit`, and `p.shift()` reaches `PosLib.shift`.
  - Fixture f8: the receiver of `a.addF(b)` on `Fixed a` became `Fixed`.
  - The README language table says "—" for Solidity type inference (README.md:517), which understates this.
- **Step 4e, `using L for T`** (cross_file.rs:6811-6840): `x.f()` on a receiver that is uppercase or `typed_receiver` reaches `L.f` for every `L` that `libraries_for(T)` returns, capped by `--fanout-cap` unless it is one overload set. Test `solidity_using_for_reaches_the_library_on_a_typed_receiver` (cgg/tests/resolve.rs:3218): `p.shift(1)` reaches `PosLib.shift`, not `Other.shift`. Fixture f2 `p.shift()` → `PosLib.shift` at **medium**.

Not supported:

- **Primitive receivers** (`uint n; n.touch()` with `using AnyLib for *`): `uint` is lowercase, so the receiver is untyped and the site goes to step-5 name fan-out. Fixture f2: `AnyLib.touch` at medium. It is right only by luck of the name.
- **Local variable types** `Other o = new Other(); o.ping();` → no `local_types` are recorded. The site is fanned out by name: fixture f1 `Other.ping` at medium (unchanged in 0.13.3, `lang-docs/solidity/a`).
- **State-variable types** `IVault public other; other.peek()` → no `class_fields`. Fixture f2: fanned out to **both** `Vault.peek` and `IVault.peek`.
- **Casts and constructions** (`IVault(addr).f()`, `new C().f()`): no `typed_receiver`. Solidity has no `CAST_INTRINSICS` or `CONSTRUCTING_METHODS` entries (cgg-core/src/stdlib.rs:35-46, 58). See §03.
- **Return types, closure parameters (`scoped_types`), implicit receivers, weak receiver types and `declared_type` wrapper tables**: no Solidity entries (grep).
- **Scoped vs file-wide locals**: the locals table (§07) only shadows; it carries no types.

## 06. Inheritance, dispatch & overloading

Supported:

- **Own-scope bare calls**: a bare call inside a contract or library that declares the name binds to that scope's same-file functions (all overloads) before any import (cross_file.rs:5681-5711). Test `solidity_bare_calls_never_reach_an_imported_library_member` (cross_file.rs:14484). Fixture f1: `hook()` inside `Token` reaches `Token.hook` (medium), not `Base.hook`.
- **Library members are never bare-reachable from outside their library**: the `library-member` filter (cross_file.rs:2659-2680) applies to receiver-less sites. Fixture f3: `lf(1)` from contract `C` is unresolved, while `L.lf(2)` binds `high`. CHANGELOG.md:180-186.
- **Overload sets**: Solidity is in `binds_same_name_set` (intra_file.rs:847), so a same-file, same-QN set is one callee. With no arity, every overload binds at **medium**. Fixture f1: `over(1)` and `over(1, 2)` each bind both `Token.over` overloads at medium.

Partial:

- **Inherited members**: `contract C is A { … only(); }` reaches `A.only` at `high` (fixture f3), but only because `A.sol` is imported and holds the sole `only`. The plugin records no bases: the `inheritance_specifier` is ignored and nothing is written to `classes` or `base_types`. A same-named member of an unrelated imported contract is therefore bound too. Fixture f3: `common()` in `C is A` bound to both `A.common` and `B2.common` (B2 is not a base) at medium.
- **`super.f()`**: binds by name via `cross-file:imports` at medium (fixtures f1, f2). It does not go through `resolve_via_bases` and is never refused as `super-base-out-of-graph`.

Not supported:

- **`--dynamic-dispatch` / `inheritance_fanout`**: Solidity is not in `INHERITANCE_LANGS` (dispatch.rs:110-123), and no `trait_impl_target` is set. Fixture f2 with `--dynamic-dispatch --reference-edges`: no `dyn` edges, and the interface `IVault.deposit` gets no edge to `Vault.deposit`.
- **`super.f()` with no base declared in the file**: refused as `super-base-out-of-graph` in 0.13.3 (fixture `lang-docs/solidity/b`: `contract U { … super.f(); }`). With a base (`contract C is B`), `super.f()` still binds `B.f` by name at medium (`lang-docs/solidity/c`).
- **`member_of_calling_class` / `overrides_below`, linearisation (C3) and `virtual`/`override` reading**: none. `override_specifier` is not read, so there are no `OVERRIDE_MARKERS` attributes.
- **Extension methods**: the analogue is `using L for T` (§05).

## 07. Scoping & shadowing

- **Locals table (WS27, 0.11.1).** `locals::collect_solidity` (`crates/cgg-lang/src/locals.rs:2671`) records the parameters and named returns of functions, modifiers, constructors, `fallback` and `receive` over the definition (never a function type's own parameters, an event's or error's), block-scope and tuple declarations from the end of their statement to the end of their block, `for` initializers over the loop, `try … returns` names over the success block and `catch` parameters over their clause. State variables are members, not locals; a member call is never shadowed; Yul is not read. Fixture (2026-11-25, `lang-docs/solidity/a`): `function shadow(function() internal helper) internal { helper(); }` is now `shadowed-by-local` (it bound `high` to `S.helper` in 0.11.0, fixture f9).
- **Intra-file binding ignores contract boundaries** (still true in 0.13.3, fixture `lang-docs/solidity/b`: `E.run -> C.freeA` high). Contract `C.run` calls the free function `freeA()` (declared in an imported file), while another contract `D` in the **same** file declares `freeA`. The call binds **high** to `D.freeA` (fixture f3, `C.run -> D.freeA high intra-file`). That is a wrong certain edge: `D` is neither a base of `C` nor visible to it by bare name.
- **Lexical narrowing and positional shadowing**: not applicable or not implemented. Solidity is not in the narrowing list in CLAUDE.md, and there are no nested functions.
- **Conditional definitions**: not applicable (no preprocessor).

## 08. Macros, preprocessor & codegen

- Not applicable: Solidity has no macros or preprocessor. `pragma` is ignored by the walker.
- No `macro:<name>` tagging, and no `name_mentions` are recorded (the plugin never writes `facts.name_mentions`).
- **Inline assembly / Yul**, the closest "embedded language", is not extracted (§02, §03).

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: no Solidity coverage (grep of `ffi.rs` finds no `solidity`). Contract ABI links to JS/TS or Python callers (ethers, web3, wagmi typechain bindings) are not modelled.
- **Descriptor links**: none.
- **Markup code-behind**: none.
- `<external>` exit nodes (`--include-external`) are the only trace of calls that leave the tree: `abi.encodeWithSignature`, `address(v).call` and `ERC20(addr).transfer` (fixture f2).

## 10. Frameworks & entry points

The only rule that applies to Solidity:

- **`solidity-public`**, kind `TrustKind::Public` (`public` in output), `node: true` (rules.rs:1416-1438).
  - **Trigger**: a definition whose native `visibility` string is `public` or `external` (`visibility_entries_for`, rules.rs:11686-11691), matched case-insensitively by `match_visibility` (frameworks/mod.rs:1654-1685).
  - It has no detect gate, no attributes, no registrars, no base types and no methods. It is whitelisted as marker-only in `every_rule_is_either_detectable_or_a_declared_gap` (rules.rs:11966-11980).
  - **Output**: one synthetic node `<framework-entry>::public::solidity-public::<name>` per simple name, with an `entry` edge at `low` to every matching definition. Interface members are included: fixture f1 has `transfer` pointing at both `IToken.transfer` and `Token.transfer`. Evidence string: "`external` visibility".
  - **Test**: `solidity_visibility_is_the_trust_boundary` (cgg/tests/frameworks.rs:661). `public` and `external` are entries; `internal` and `private` are not.
  - **Benchmark claim**: `solidity-openzeppelin` APPS entry (scripts/benchmark.sh:171). CHANGELOG.md:4983-4986 records 1,495 entries on OpenZeppelin and a 37% drop in dead-code false positives.
- **Stale comment**: frameworks/mod.rs:1646-1648 says Solidity "leaves the latter [`vis`] `Unknown`". solidity.rs:210-220 has populated `vis` since that note was written.
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `observer_types` and registrar verbs**: none for Solidity (every `RuleSpec` carries one `language`, and `solidity` appears only in the rule above; grep of rules.rs).
- **Not modelled**:
  - `fallback`/`receive` (no callables, §02);
  - OpenZeppelin hook conventions (`_beforeTokenTransfer`, `_update`, `_authorizeUpgrade`), invoked by a base contract — only reachable through ordinary edges if the base is in the tree;
  - modifiers as entry wrappers (§03);
  - Chainlink/Gelato keeper callbacks (`checkUpkeep`/`performUpkeep`), which are public anyway;
  - deploy scripts (Hardhat/Foundry `script/*.s.sol` `run()`), which are also only rooted because they are public.

## 11. Test harness entries

- **No test harness rules** (no `kind: test` rule with a Solidity language; grep of rules.rs for `foundry`, `forge` and `hardhat` finds nothing). Foundry `function test_x() public`, `testFuzz_*`, `invariant_*` and `setUp()` are rooted **only** through `solidity-public`, because they are public. Fixture f7: `BoxTest.setUp`, `test_set` and `testFuzz_x` get `solidity-public` entries, not a `<framework-entry>::test::…` harness node.
- **Test-file classification**: since 0.11.1 (WS20) `classify_test_file` has a Solidity arm, `*.t.sol` (`crates/cgg-core/src/testfile.rs:138`). Fixture (2026-11-25, `lang-docs/solidity/a`): `test/Box.t.sol` has `test_role: "file-name"`. Fixture f7: the `internal` `BoxTest.helperT` in `test/Box.t.sol` is reported only with `--include-tests`.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** is `["python", "php", "ruby"]` (rules.rs:11806); Solidity is not in it.
- **`test_defs` signal**: not declared (solidity.rs:21-26).

## 12. Dead-code analysis

- **Signal coverage**: `vis yes`, `refs no`, `disp no`. The `attrs` column is `yes` only when a `library-member` attribute was observed in the tree, because `measure` upgrades on observation (deadcode/caps.rs:130-133, 157-163).
  - With no library in the tree, the language class is **`degraded`** (fixture f7: `solidity 2 7 degraded yes no no no`).
  - With one, it is `analyzable` (fixture f1).
- **Roots**:
  - `solidity-public` entries (§10).
  - The universal `main` (roots.rs:20).
  - No `LANG_ENTRY` row for Solidity (roots.rs:23-32).
  - Constructors are **not** roots. Fixture f1: `unreferenced constructor 'Token.constructor'` at medium, with risk `implicitly-invokable-kind`. Callees reachable only from a constructor are also reported (`Token.init`, `incoming-only-from-dead-code`). Since 0.11.1 a constructor reached by `new C(…)` or a constructor-header base call has an incoming edge.
- **Visibility evidence**: `Vis::Private` **and `Vis::Internal`** both add `private-visibility` (corroborating) evidence (deadcode/mod.rs:951-953). In Solidity, `internal` means "this contract and every contract deriving from it". For a library-style codebase (OpenZeppelin), an `internal` function of an abstract contract is exactly what out-of-tree derived contracts call. It is treated as unit-confined anyway (fixture f1 `Helper.go`, internal: `private-visibility`).
- **Findings in fixture f1 (all medium)**:
  - `Base.hook`: a bodiless `virtual` declaration that `Token.hook` overrides, with no dispatch model;
  - `Token._check`;
  - `Token.init`;
  - `Token.dead`;
  - `Token.onlyOwner`: a modifier, because modifier invocations are not recorded (§03), so **every modifier is reported dead**;
  - `Token.constructor`;
  - `Helper.go`.

  Each carries the risk tags `language-lacks-dispatch-model` and `language-lacks-value-references`; fixture f7 adds `language-lacks-attributes`.
- **Caps**: no `implements-external-contract` cap. `override` is not read, there are no `OVERRIDE_MARKERS`, and Solidity is not special-cased in `external_contract` (deadcode/mod.rs:470-530). An override of an OpenZeppelin hook whose base is outside the tree is therefore not capped. No `macro-generated` cap and no `reflection` cap.
- **Unreachable-after-terminator**: not declared (`unreachable` signal off).
- **Unresolved-doubt handling**: generic. A same-named unresolved site adds `name-matches-unresolved-site` (fixture f1 `Helper.go`, because of the unresolved `H.go()`).

## 13. Builtins & stdlib tables

- **There is no `stdlib/solidity.txt`** (`ls crates/cgg-core/src/stdlib/`), and `stdlib_names` has no `solidity` row (stdlib.rs:220-260). As a result:
  - `require`, `assert`, `keccak256`, `abi.encode*`, `address(x).call` and `type(C)` are never classified `stdlib`; they are `external` (fixture f1: `<external>::require`; f2: `<external>::abi::encodeWithSignature`). `--include-stdlib` adds nothing for Solidity (fixture f1: `stdlib_calls: 0`).
  - Step-5 name fan-out has no stdlib exclusion. For example, an untyped `x.add()` fans out to any workspace `add` (fixture f1: `total.add(amt)` → `Math.add` at medium, correct only by coincidence).
- **`--externals` (0.12.0)** attributes imports: row `solidity` in `crates/cgg/tests/externals.rs` attributes `import "@openzeppelin/contracts/…"` to `third-party` `@openzeppelin/contracts`. It never changes an edge.
- **`CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, implicit packages and `rust_types`-like wrapper tables**: no Solidity entries (stdlib.rs:35-58).
- **Globals** (`msg.sender`, `block.timestamp`, `tx.origin`): member reads, not calls, so they are not recorded. That is correct.

## 14. Confidence & certainty

`high` is produced by:

- An intra-file single candidate (fixture f1 `onlyOwner -> _check`, `constructor -> init`, `this.peek()`). This includes the **wrong** cross-contract edge `C.run -> D.freeA` (§07); the shadowed `helper()` is fixed since 0.11.1.
- A constructor reached by `new C(…)` or a base-constructor header call (0.11.1).
- A cross-file import-anchored single candidate (fixture f3 `A.only`, f6 `afn`, f4 `toStr`).
- An uppercase library/contract qualifier (`Math.mul`, `L.lf`, `Lb.Math.mul`).
- A typed parameter receiver with one owner (fixture f2 `v.deposit -> IVault.deposit`, `try v.peek()`).

`medium` is produced by:

- overload sets (always, because there is no arity);
- by-name fan-out on untyped receivers;
- `super.f()`;
- `using … for` step-4e binds (fixture f2 `p.shift`);
- own-scope binds where intra-file found several candidates (fixture f1 `hook()`).

Other notes:

- `low`: only framework-entry edges and exit nodes. `--guess-unique-names` is Rust-only.
- `CertainCtx`/`OwnerFacts`: there is no Solidity-specific path. Typed-owner `high` relies on the generic owner index and on the type name being unique.
- Mermaid `?` marker: generic. Any arrow with no `high` site is prefixed `?` (CLAUDE.md, Phase 3 notes). Not exercised specifically for Solidity.

## 15. Dynamic use & reflection

- **`dyn_uses` signal**: off (solidity.rs:21-26). `plugins/dynuse.rs` has no Solidity code (grep).
- **String-selector dispatch is not screened**: `abi.encodeWithSignature("deposit(uint256)")`, `abi.encodeWithSelector(this.f.selector)`, low-level `address(x).call(...)` / `delegatecall` / `staticcall`, and proxy `fallback` forwarding. None of them caps a same-named dead-code finding or creates an edge (fixture f2: only exit nodes `address(v)::call` and `abi::encodeWithSignature`).
- **`this.f.selector` / `type(I).interfaceId`**: not recorded.
- **Module loads**: not applicable.

## 16. Known gaps & limitations

Collected from the sections above (fixture evidence in each).

Wrong `high` edges:

1. Intra-file binding crosses contract boundaries (`C.run -> D.freeA`, fixture f3).
2. ~~A function-typed parameter shadowing a function binds to the function (fixture f9).~~ Fixed by the 0.11.1 locals table (§07).

Missing call kinds:

1. Modifier invocations, inheritance-list base-constructor arguments (`is Base(1)`), `emit` and `revert` are not call sites. Every modifier appears dead. (`constructor() Base(2)` is a call since 0.11.1.)
2. ~~`new C(...)` is recorded as the name `"new C"`~~: a constructor call since 0.11.1.
3. `fallback()`/`receive()`, public state-variable getters, events, errors and structs are not callables. Yul assembly functions and calls are ignored.

Imports:

1. Import aliases (`import * as L`, `import "x" as L`, `{A as B}`) are dropped, so aliased calls are unresolved. Named imports do not narrow.
2. No import remappings (`@openzeppelin/…`, `forge-std/…`, `remappings.txt`, `foundry.toml`), and no `external-module` classification for unresolvable imports.
3. Imports are not followed transitively; such calls bind by name at medium, including to unimported files (fixture f6).

Typing, inheritance and dispatch:

1. No base-contract records: inherited calls are not restricted to bases, `super` is name-matched, and there is no `--dynamic-dispatch`/`inheritance_fanout` (not in `INHERITANCE_LANGS`). (Interface members are tagged `abstract` since 0.11.1.)
2. No local, state-variable, cast or construction typing. Only uppercase-typed parameters are typed. `using L for *` on primitive types relies on name fan-out.
3. `using {f, g as +} for T global` (brace form) is not recorded.
4. No arity, so every overload of a set binds at medium.

Builtins, tests and dead code:

 1. No stdlib table (`require`, `keccak256` and `abi.*` are `external`, never `stdlib`), and no stdlib guard on name fan-out.
 2. No Foundry/Hardhat test harness rule (`.t.sol` is a test file since 0.11.1).
 3. Dead code: constructors are not roots. `internal` is treated as unit-private although derived contracts outside the tree can call it. No `implements-external-contract` cap for `override`. The language drops to `degraded` when the tree has no `library`, because `attributes` is not declared.
 4. No dynuse screening of selector/low-level-call dispatch.

Stale prose:

 1. frameworks/mod.rs:1646-1648 (says `vis` is left `Unknown`) and README.md:517 (says no type inference).

Other notes:

- Receiver text is the raw object text: casts and chained calls produce receivers like `Other(address(0))`, and a binary-expression receiver was garbled in fixture f8.
- There are no `#[ignore]` tests and no `gap:` text for Solidity (`solidity-public` has `gap: ""`, rules.rs:1434).
- CHANGELOG follow-ups name no open Solidity item; the Solidity entries (CHANGELOG.md:180-186, 953-954, 259, 1464, 2015, 4983) record fixes already shipped.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | full | `.sol` by extension, one crates.io grammar; no re-parse, no remapping/manifest reading |
| 02 Callables extracted | partial | functions, modifiers, constructors, library/interface/free functions; no fallback/receive, getters, Yul |
| 03 Call-site forms | partial | bare/member/qualified/`this`/`super`, `new C()` and constructor-header base calls; modifiers, `is Base(x)`, emit/revert, value refs and arity missing |
| 04 Imports & module resolution | partial | relative and suffix-matched `import` paths plus `using-for`; no aliases, remappings, transitivity or external-module |
| 05 Type inference & receiver typing | minimal | uppercase-typed parameters and `using L for T` only; no locals, state vars, casts or constructions |
| 06 Inheritance, dispatch & overloading | partial | own-scope rule, library-member gate, overload sets (medium); no bases, super chain or dispatch fan-out |
| 07 Scoping & shadowing | partial | locals table (parameters, block declarations); intra-file binding still crosses contracts (wrong `high`) |
| 08 Macros, preprocessor & codegen | n/a | the language has none; Yul assembly ignored |
| 09 FFI & cross-language links | none | no ABI/typechain links; only `<external>` exit nodes |
| 10 Frameworks & entry points | partial | single marker rule `solidity-public` (public/external); no fallback/receive/hook conventions |
| 11 Test harness entries | minimal | Foundry tests rooted only as public functions; `*.t.sol` is a test file |
| 12 Dead-code analysis | partial | visibility-driven; modifiers and constructors falsely dead, `internal` treated as private, class can drop to degraded |
| 13 Builtins & stdlib tables | none | no `stdlib/solidity.txt`; builtins all `external` |
| 14 Confidence & certainty | partial | generic `high` paths only, including some wrong ones; overloads always medium |
| 15 Dynamic use & reflection | none | selector/low-level-call dispatch not screened |
| 16 Known gaps & limitations | n/a | gaps listed above; the cross-contract intra-file binding still produces wrong `high` edges |
