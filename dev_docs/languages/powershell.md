# powershell

The PowerShell plugin is `PowerShellPlugin` in `crates/cgg-lang/src/plugins/powershell.rs`. The file is 548 lines, including 9 inline tests at lines 401-548.

- **Grammar**: the crates.io crate `tree-sitter-powershell`, workspace pin `"0.26"` (Cargo.toml:114, crates/cgg-lang/Cargo.toml:53), loaded as `tree_sitter_powershell::LANGUAGE` (powershell.rs:21-23). Nothing is vendored.
- **Walker**: a single recursive `PowerShellWalker` (powershell.rs:44-399) acts on five node kinds (powershell.rs:62-101):
  - `function_statement` becomes a definition.
  - `class_statement` pushes the class name as scope.
  - `class_method_definition` becomes a method, constructor or static method.
  - `command` becomes a bare reference, or an import for `using`, `Import-Module` or a dot-source.
  - `invokation_expression` becomes a member call with a receiver hint.
- **Optional hooks**: the plugin overrides none of `signals()` (so every `PluginSignals` bit is false, lib.rs:304-306), `max_line_bytes` or `ts_language_for`. It ignores `ExtractCtx` (`_ctx`, powershell.rs:27), so reference sites and registrar verbs are never recorded.
- **Resolver kind**: the plugin trait has no `resolver_kind` method in this tree; the task's checklist names one, but `grep` finds none in `crates/cgg-lang/src/lib.rs`. Resolution uses the generic pipeline.
- **PowerShell-specific resolver code**: only the import arms at cross_file.rs:2090-2122:
  - `using-*` kinds become namespace-open prefixes.
  - `import-module` and `dot-source` are include-like, followed through `collect_include_defs` with budget 4 and trying `path`, `path.psm1` and `path.ps1`.
- **Dead-code class**: the language is listed as `SCRIPT_DRIVEN` (crates/cgg-resolve/src/deadcode/caps.rs:27-36).

Fixtures cited below live under `<scratch>/parity/powershell/` (f1 … f14, bis, bis2). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`. The results were dumped with `show.py` (graph) and `aud.py` (audit sidecar) in the same directory.

## 01. Detection & parsing

- **Extensions `.ps1`, `.psm1`, `.psd1`** (powershell.rs:15-17).
  - f2: `Mod.psm1` and `Mod.psd1` report `extension:.psm1` / `extension:.psd1`.
  - Matching is case-insensitive. f9: `x.PS1` → `extension:.ps1`.
- **Shebangs `pwsh`, `powershell`** are declared (powershell.rs:18-20) but never win. Detection checks shebang needles with `word.contains(needle)` in plugin registration order (detect.rs:63-75). Bash is registered earlier, PowerShell at plugins.rs:155, and both `pwsh` and `powershell` contain `sh`.
  - f9: `#!/usr/bin/env pwsh` (extensionless `script`), `#!/usr/bin/pwsh -NoProfile` (`s3`) and `#!/opt/microsoft/powershell/7/powershell` (`s4`) are all detected as `bash` via `shebang:sh`.
  - Even `s2.ps1` with a `pwsh` shebang is routed to **bash**, because the shebang rule runs before the extension rule (detect.rs:63 vs 100+).
  - This is a real misrouting bug: the PowerShell file is parsed by the Bash grammar.
- **Content detection**: none. The only content sniff is the YAML/JSON descriptor sniff (detect.rs:77-97).
- **Grammar**: one grammar for all three extensions. `.psd1` manifests parse as a hashtable expression and contribute nothing (f2: `Mod.psd1` gives `parse_status ok`, 0 callables, no facts).
- **Constructs that put the file in `parse_status: error`** with this grammar. Extraction proceeds on the error-recovered tree, with no re-parse or fallback.
  - Constructor chaining `S([string]$n) : base() { … }`: bis2/a.ps1 → `error`. The constructor is still extracted.
  - A call after an inline param block on the same line, `function A { param($Name) B }`: f8 → `error`, and the call `B` is **lost entirely** (no edge, no unresolved record). The same body split over lines (`function C`) yields `C -> B high`.
  - Script invocation by relative path, `./b.ps1 -X 1` inside a function: f13/a.ps1 → `error`.
  - DSC `configuration MyConfig { Node localhost { … } }`: f14 → `error`. `configuration`, `Node` and `File` are recorded as external commands.
- **Depth skip**: the shared `MAX_TREE_DEPTH` (4,000) / `SkipReason::TooDeep` gate applies (cgg/src/lib.rs, per CLAUDE.md). There is no PowerShell-specific handling.
- **Long lines / minified**: `max_line_bytes` is not overridden (default `None`, lib.rs:313-315).
  - f13/long.ps1, a single 3.2 KB line with 800 calls, is analyzed normally.
  - There is no locals spec (locals.rs:380-393), so the 2,000-byte locals rule never applies.

## 02. Callables extracted

Every definition has `visibility: ""`, `attributes: []`, and a `signature_hint` from `super::extract_signature` (text up to the first depth-0 `{`, plugins.rs:74).

- **`function Name { … }`** → `FreeFunction`, named by its `function_name` (powershell.rs:130-160). Test `free_function`; f1 `Get-Greeting`.
- **`function Name($a) { … }`** (parenthesised parameters) → `FreeFunction`, with `signature_hint` `function Format-Name($Value)` (f1).
- **`filter Name { … }`** → `FreeFunction` (it is a `function_statement`). Test `filter_is_callable`; f1 `Double`.
- **`workflow Name { … }`** → `FreeFunction` (f1 `Run-Flow`).
- **Nested `function`** inside a function → a separate `FreeFunction` with a **flat** name. The scope stack is only pushed by classes, so nesting is invisible in the name. f1: `Inner` is `Inner`, not `Outer.Inner`.
- **Scope-modified names** `function global:Gf` / `function script:Sf` keep the prefix in the name: `global:Gf`, `script:Sf` (f14). A bare call `Gf` then does not match (see 03).
- **Class methods** `[void] Run() { }` → `InherentMethod`, with qn `Class.Method` (`self.scope.join(".")`, powershell.rs:54-60, 162-219). Test `class_methods`; f1 `Service.Run`.
- **Static methods** `static [Service] Create()` → `StaticMethod`, detected by the `class_attribute` text containing `static` (powershell.rs:172-176, 196-197). Test `class_methods`.
- **`hidden` methods** → `InherentMethod`. `hidden` is not recorded as visibility (f1 `Service.Helper`).
- **Constructors** (method named like the enclosing class) → `Constructor` (powershell.rs:192-195). Test `class_methods`; f1 `Base.Base`, `Service.Service`.
- **Operator-named methods** such as `static [P] op_Addition(...)` are an ordinary `StaticMethod` (f14). There is no operator modelling: `$a + $a` produces no call.
- **Accessor-style methods** such as `get_X2()` are ordinary methods (f14).
- **Not extracted**:
  - **Class properties** (`[string]$Name`) and **enums** (`enum Color { Red; Green }`): f1, no callables.
  - **Script blocks** (`$sb = { param($x) … }`, `{ }` passed to `-ScriptBlock`, Pester `It { }`). No closure callables exist; calls inside them are attributed to the enclosing function, or to none at top level (f1 L28, f7).
  - **The script top level** (the body of a `.ps1`, including Azure Functions `run.ps1` with `param($Request, …)`). Calls there have `src: null`, reason `no-enclosing-callable` (f1 L28-48, f7, f12/b.ps1).
  - **DSC `configuration` blocks** (f14).
  - **Interface members**: PowerShell has none, and class base lists are not read.
  - **Generated code**: none.
- **Overloads / redefinitions**: two `function Dup` in one file → two callables with the same qn (f10/b.ps1). PowerShell's "last definition wins" rule is not modelled (see 06/07).
- **Qualified names**: free functions are their bare name, and class members are `Class.member` with no file or module prefix (powershell.rs:54-60). `simple_name` is the member name.

## 03. Call-site forms

The plugin records two shapes: `command` (bare reference, no receiver; powershell.rs:221-287) and `invokation_expression` (member name plus a receiver hint; powershell.rs:349-398). `RefRecord::arity`, `paren_less`, `value_ref`, `path_receiver`, `typed_receiver` and `arg_type` are never set.

**Supported**

- **Cmdlet/function command** `Get-Greeting -Name "world"` → bare ref `Get-Greeting`. Test `cmdlet_call_captured`; f1 L5 `Get-Greeting -> Format-Name high intra-file`.
- **Commands nested in expressions** are all walked, because `walk_children` recurses (powershell.rs:62-101). Each of these is recorded (f9 L15-18, f12 L10-13):
  - pipelines `A | B`;
  - subexpressions `$(Q)`, `"$(Q)"`;
  - parenthesised `(Q)` and `-Other (Q)`;
  - `if (Q) { Q }`, `return (Get-Greeting)`;
  - script-block arguments `-ScriptBlock { Get-Greeting }`;
  - `1..3 | % { Q }`.
- **Call operator with a bare name** `& Q`, and **call operator on a script block** `& { Q }`: both record `Q` (f9 L6, L8).
- **Member call on a variable** `$svc.Run()` → ref `Run`, receiver `svc` (`$` stripped). Test `method_invokation_captured`; f5.
- **`$this.M()`** → receiver `this` (f1 L22, f11).
- **Static call** `[Service]::Create()` → ref `Create`, receiver `Service` (the `type_spec` text, powershell.rs:370-380). Test `method_invokation_captured`; f5 L11 high.
- **Chained call** `[Service]::Create().Stop()` → two refs:
  - The inner one keeps receiver `Service`.
  - The outer one gets an **empty** receiver, because the first child is an `invokation_expression`, not a variable or type. It therefore resolves as a bare name. f5 L15: `Use-Chain -> Service.Stop high`, correct only because the name is unique.

**Partial or wrong**

- **Constructor calls** `[Service]::new(...)` are recorded as ref `new` with receiver `Service`. They never reach the declared `Service.Service` constructor; the result is `external` (f1 L24/L34, f9 L4 with `Svc()` declared). `New-Object Service` is a command ref `New-Object` → external (f5 L12).
- **Cast receiver** `([Base]$this).Hello()` → empty receiver (parenthesised), so it resolves as a bare name.
  - bis2/c.ps1 binds it to **itself** (`S.Hello -> S.Hello high`), which is a false high.
  - f1 refuses it as `ambiguous-in-file`.
- **Dynamic member** `$x.$m()` → ref named literally `$m`, external (f9 L13).
- **Pipeline variable receiver** `$_.Stop()` → receiver `_`. It is recorded both as unresolved (`candidates-in-other-files`) and as external `<external>::_::Stop` (f5 L16).
- **The `1..3` range** at the head of a pipeline is recorded as a *command* named `1..3` → external (f1 L41, f9 L18). This is grammar mis-classification taken literally.
- **Aliases** such as `%` and user `Set-Alias gg Get-Greeting` → `gg` / `%` stay literal and go external. Aliases are never expanded (f1 L46-47, f9 L18).
- **Module-qualified command** `Mod\Invoke-Mod`, `Microsoft.PowerShell.Utility\Write-Host` → the name keeps the `Mod\` prefix → external (f2 L12, f13/c.ps1).
- **Case**: PowerShell command names are case-insensitive, but matching is case-sensitive. `get-greeting` / `GET-GREETING` → external although `Get-Greeting` is defined in the same file (f12 L4-5).
  - Type literals behave the same way. `[svc]::Run()` bound to `Svc.Run` only by bare-name luck (f12 L8).
- **Scope-qualified call** `global:Gf` matches the def `global:Gf`, but bare `Gf` does not (f14).

**Not recorded at all**

- `& "Q"` / `& 'Format-Name' z` (quoted call-operator target): f9 L7, f1 L31.
- `& $sb` / `& $fn` (variable target): f1 L29, L45.
- `& "$PSScriptRoot/b.ps1"`, and `./b.ps1` script invocation: f12, f13.
- `${function:Q}` function-provider references, and `Invoke-Command -ScriptBlock ${function:Q}`: f9 L10-11.
- Property access `$x.Go` (correctly not a call): f9 L14.
- Dot-sourcing a *function* (`. Q`) is recorded as a dot-source *import* of path `Q`, not as a call (powershell.rs:266-278; f9 L9 produced no edge).
- Value references and registrar arguments: `ExtractCtx` is ignored, so `--reference-edges` has nothing to emit. `Register-ArgumentCompleter -CommandName Get-Greeting` → only the external `Register-ArgumentCompleter` (f12 L11).
- Arity: never recorded.

## 04. Imports & module resolution

**Import records produced by the plugin**

- **`Import-Module X`** → `ImportRecord { kind: "import-module", path: X }`. The path is the first `generic_token` / `command_name` element, with quotes stripped (powershell.rs:240-265, 324-347). Test `import_module_captured`.
  - `Import-Module -Name X`: unverified. The first element is probably the `-Name` parameter token, which is a `command_parameter` and is skipped by `first_command_argument`, so this likely works.
- **`Import-Module -Name (Join-Path … -ChildPath 'X')`** → path `X`, taken from the `-ChildPath` literal by text search (powershell.rs:245-254). Test `computed_import_module_path_records_the_child_path`; CHANGELOG.md:1929-1930 (NetworkingDsc, 82 DSC entries).
- **Dot-source** `. .\file.ps1` → `kind: "dot-source"`, with the name as the path (powershell.rs:266-278). Test `dot_source_captured`.
- **`using namespace|module|assembly X`** → `kind: "using-namespace"` / `"using-module"` / `"using-assembly"` (powershell.rs:234-238, 289-322). Test `using_namespace_captured`.

**How the resolver uses them**

- `using-*` → pushed onto `unqualified_prefixes` (cross_file.rs:2090-2102). Step 1b then tries `<prefix>.<name>` / `<prefix>::<name>` (cross_file.rs:5943-5986). Class qns carry no namespace, so this can only match a module-like prefix; no fixture produced a 1b hit.
- `import-module` / `dot-source` → `collect_include_defs` with budget 4, trying `path`, `path.psm1` and `path.ps1` relative to the includer's directory, then a path-suffix match (cross_file.rs:2103-2122, 3385-3505).
  - Matched files' definitions go to `direct_imports`. The path is also pushed as an unqualified prefix.
  - **Not transitive**: the transitive arm only follows kinds `include` / `system-include` (cross_file.rs:3491-3504). A dot-sourced file's own dot-sources are not followed.
  - **Forward-slash paths resolve**. f3: `. ./lib/helper.ps1` → `Main -> Get-Help2 high` and `Main -> Shared high` (lib copy only, although `other/dup.ps1` also defines `Shared`).
  - **Bare module names resolve**. f3: `Import-Module Mod` → `Mod/Mod.psm1` via the last-segment index. `Main2 -> Invoke-Mod high` and `Main2 -> Get-Private high`. The second is wrong in spirit, because `Get-Private` is not exported (see below).
  - **Backslash (Windows) paths do not resolve on a POSIX host.** `.\lib\helper.ps1` is one path component, so no file matches (f2, f4/c.ps1). The calls then fall to step 1d (below): `Get-Help2` medium, and `Shared` fans to **both** copies. Backslash paths are the norm in PowerShell sources.
  - `Import-Module .\Mod\Mod.psd1` (manifest path) has the same backslash problem, and the manifest's `RootModule` is never read (f2).
  - Interpolated paths `"$PSScriptRoot\lib\helper.ps1"` → recorded verbatim. Unverified whether `$PSScriptRoot` is ever stripped; the code does no such stripping (powershell.rs:266-278).
- **Step 1d global by-simple fallback.** Any import at all, even `Import-Module Nonexistent` or `using namespace System.IO`, enables a workspace-wide bare-name lookup with ≤3 candidates (cross_file.rs:6015-6060). It binds at **medium** and binds every copy.
  - f4: `a.ps1` (`Import-Module Nonexistent`) and `b.ps1` (`using namespace`) both get `Lonely` medium and `Shared` ×2 medium.
  - A file with **no** import refuses the same calls as `candidates-in-other-files` (f3/main3.ps1, f4/d.ps1).
  - So cross-file linking of bare cmdlets depends on an unrelated import being present.
- **Exports are not modelled.** `Export-ModuleMember -Function X` is recorded as an external command (f2 Mod.psm1 L3). The `.psd1` `FunctionsToExport` list is not read (f2). Private module functions bind from importers (f3 `Get-Private high`).
- **Module-qualified calls** `Mod\Fn` → external (03).
- **`FileFacts::packages`**: never populated by this plugin.
- **heads.rs external-module refusal / `WorkspaceVocab`**: PowerShell is not among its languages (heads.rs has no `powershell` string). Built-in cmdlets go external only through the generic `no-candidate-in-file` → `name_screen_applied: external` path (f1 `Write-Host`).
- **modules.rs `not-in-imported-module`**: not applicable. `modules.rs` has no PowerShell handling (grep finds none).
- **libscope.rs**: no PowerShell scope.

## 05. Type inference & receiver typing

- **Type-literal receivers** `[Svc]::M()` are typed by their written type. They bind high intra-file (f5 L11, L17 `Use-Static2 -> Other.Run high`) and high cross-file via a dot-source (f9 `U1 -> Svc.Make high`).
- **Variable receivers are never typed.** `type_hints.rs` has no PowerShell handling, and the plugin records no locals, parameter types or `local_types`. f5, all with two classes defining `Run`:
  - `function Use-Param([Service]$svc) { $svc.Run() }` → fans to `Service.Run` **and** `Other.Run`, medium.
  - `$s = [Service]::new(); $s.Run()` → same fan-out, medium.
  - `$s = [Service]::Create(); $s.Stop()` → medium (unique name). The declared return type `[Service]` is not used.
  - `$s = New-Object Service; $s.Run()` → fan-out, medium.
  - `[Service]$s = Get-Thing; $s.Run()` (typed local) → fan-out, medium.
- **`$this`** → receiver `this`, untyped. f11: `$this.Hello()` in `Kid` fans to `Base.Hello` and the unrelated `Other.Hello` (medium). In bis2/e.ps1 it bound to a `S.Hello` in a **different file** (medium).
  - When the name is unique in the caller's file, it binds high intra-file (f1 `Service.Run -> Service.Hello high`, f11 `Base.Tmpl -> Base.Hook high`).
- **Casts / constructions**: `CAST_INTRINSICS` / `CONSTRUCTING_METHODS` / `RefRecord::typed_receiver` are not used. `[T]$x` and `[T]::new()` give no type.
- **`declared_type` nullability/wrappers, closure-parameter typing (`scoped_types`), implicit receivers, `weak_receiver_type`**: not applicable. The plugin records none of the inputs.

## 06. Inheritance, dispatch & overloading

- **Base classes are not recorded.** `class Service : Base` / `class S : B, IDisposable` produce no `ClassDecl` and no bases (powershell.rs:69-82 reads only `simple_name`).
  - Inherited calls fan by name. f11: `Kid.Go` `$this.Hello()` → `Base.Hello` + `Other.Hello` medium.
  - `resolve_via_bases`, `member_of_calling_class` and `one_declaring_ancestor` never apply.
- **`base` calls**: constructor chaining `: base()` breaks the parse (01). `([Base]$this).M()` is unmodelled (03).
- **Overrides**: `Kid.Hook` overriding `Base.Hook` is not linked to `$this.Hook()` in `Base.Tmpl`, even with `--dynamic-dispatch` (f11 `out2.json` is identical). PowerShell is not in `INHERITANCE_LANGS` (dispatch.rs:110-122), and `trait_impl_target` is never set.
- **Interfaces** (`: IDisposable`): not recorded.
- **Overloads**: not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583, `["java","scala","cpp"]`), and arity is never recorded.
  - Two same-named functions in one file → `ambiguous-in-file` (f10/b.ps1 `W -> Dup` refused).
  - Same-name methods in different classes of one file → `ambiguous-in-file` for bare calls (f1 L25).
- **Duck-typed fan-out** on untyped receivers follows `--fanout-cap` (default 5). f10/a.ps1: six classes with `Run` → `fanout-cap-exceeded` (candidates 6).
- **Extension methods, mixins, traits**: not applicable to the language.

## 07. Scoping & shadowing

- **`FileFacts::locals`**: none. PowerShell has no locals spec (locals.rs:380-393), so `shadowed-by-local` never fires.
- **Lexical narrowing of nested functions**: not applied. PowerShell is not in `lexically_scoped` (intra_file.rs:119-136).
  - f10/d.ps1: `Outer2` and `Outer3` each declare a nested `Helper` and call it, and `Top` calls `Helper`. All three calls are refused `ambiguous-in-file` (file_local 2).
  - PowerShell's dynamic scoping (a nested function is visible to callees) is likewise unmodelled.
- **Same-file preference**: a definition in the caller's file wins (intra-file stage). f1/f12 bind intra-file high when the name is unique in the file.
- **Positional / last-definition-wins shadowing**: not modelled. f10/b.ps1 redefinition → ambiguous.
- **Conditional definitions** `if ($IsWindows) { function Plat {…} } else { function Plat {…} }` → two callables, call refused `ambiguous-in-file` (f10/c.ps1). `DefRecord::conditional` is not set.
- **Scope modifiers** (`global:`, `script:`, `private:`, `local:`): kept literally in the function name; the bare call does not match (f14).

## 08. Macros, preprocessor & codegen

- Not applicable. PowerShell has no macros or preprocessor.
- No `macro:<name>` tagging and no `name_mentions`.
- Aliases (`Set-Alias`, `New-Alias`) are the closest analogue and are not expanded (f1 L46-47).
- `Invoke-Expression "Format-Name x"` string code is not parsed (f1 L43, external `Invoke-Expression` only).

## 09. FFI & cross-language links

- **`ffi::link_ffi`**: no PowerShell arm (ffi.rs has no `powershell` string).
  - `Add-Type -TypeDefinition '…C#…'` is not linked (only the `Add-Type` command would be recorded). Unverified by fixture; inferred from the absence of any handling.
  - `[DllImport]` inside an `Add-Type` string: same.
- **Cmdlets implemented in C#** (`[Cmdlet(VerbsCommon.Get, "Foo")]` classes) are not linked to PowerShell call sites named `Get-Foo`. No rule or resolver maps a C# cmdlet class to its verb-noun name (grep for `Cmdlet` in rules.rs / ffi.rs finds no PowerShell bridge; unverified beyond grep).
- **Descriptor links** (`descriptor.rs`) and **markup code-behind** (`markup.rs`): not applicable.
- **Script-to-script invocation** (`./other.ps1`, `& "$PSScriptRoot/x.ps1"`): not linked; see 03.

## 10. Frameworks & entry points

Rules whose `language` is `powershell` in `crates/cgg-core/src/frameworks/rules.rs`. No multi-language rule names PowerShell.

| rule id | kind | trigger | node | effect |
| --- | --- | --- | --- | --- |
| `azure-functions` (rules.rs:4616-4636) | Network | `detect_paths: ["run.ps1"]` | `node: false`, no matchers | detect-only. Reported under `seen_no_rules` with its `gap` text; no entries (f7 HttpTrigger/run.ps1) |
| `pester` (rules.rs:6466-6481) | Test | `detect: ["Pester"]` (import prefix) | `node: true`, no matchers | detect-only. `seen_no_rules` with gap text (f7 pester/a.Tests.ps1) |
| `powershell-dsc` (rules.rs:6483-6508) | Lifecycle | `detect`: `PSDesiredStateConfiguration`, `PSDscResources`, `DscResource.Common`, `DscResource.Base`, `xPSDesiredStateConfiguration` | `node: true` | `methods`: `Get-TargetResource`, `Set-TargetResource`, `Test-TargetResource`. Mints `<framework-entry>::lifecycle::powershell-dsc::<name>` nodes with `entry` edges at **low** (f7: 4 entries) |
| `powershell-module` (rules.rs:6510-6535) | Cli | `detect`: `Microsoft.PowerShell.Utility`, `Microsoft.PowerShell.Management`, `Microsoft.PowerShell.Security`, `System.Management.Automation`, `PSScriptAnalyzer`, `PowerShellGet`, `Az.Accounts`, `ActiveDirectory` | `node: true`, no matchers | detect-only. `seen_no_rules` with gap text (f7 mod/m.psm1) |

- **Detection mechanics** (frameworks/mod.rs:264-337):
  - Import records are prefix-matched against `detect`, so `Import-Module`, `using module` and dot-source paths all count.
  - `detect_paths` is matched against the file path.
  - Activation is **per tree** (`is_active`, frameworks/mod.rs:362-365). Once any file imports a DSC module, every PowerShell `Get/Set/Test-TargetResource` in the tree becomes an entry. f7: `dsc2/res2.psm1` has no import, yet still got an entry.
- **Corpus evidence**:
  - `scripts/benchmark.sh:288` `app-importexcel-pester` (`~pester,~powershell-module`, i.e. expected detect-only).
  - `scripts/benchmark.sh:335` `app-networkingdsc-dsc` (`powershell-dsc`).
  - `scripts/benchmark.sh:90` / `386` PowerShellGet.
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for`, `detect_calls_for`**: no PowerShell entries (grep of rules.rs finds the language only at the four rules above). The plugin records no attributes or visibility, so attribute/visibility rules could not fire anyway.
- **Registrar verbs**: none (`ExtractCtx` is ignored).
- **Not supported**, each named in the rules' own `gap` text:
  - `[CmdletBinding()]` / `[Parameter()]` advanced functions;
  - `FunctionsToExport` / `Export-ModuleMember` public surface;
  - Azure Functions `run.ps1` top-level handlers and `function.json` triggers;
  - Pester `Describe`/`It` blocks;
  - also Universal Dashboard / Pode routes (`Add-PodeRoute -ScriptBlock {}`). No rule exists; unverified beyond grep.

## 11. Test harness entries

- **`pester`** (rules.rs:6466-6481) is kind Test but has no matchers. It mints no `<framework-entry>::test::pester` node and reports a gap (f7).
  - Its `Describe`/`It` script blocks are not callables.
  - Calls inside `It { … }` at file top level are unattributed (`no-enclosing-callable`, f7 L4 `Test-Helper`).
  - A function called there still becomes a `toplevel:invocation` root (f7: `Test-Helper top-level-invocation`).
- **The generic test-name rule misfires.** `is_test_name` falls to the default arm `name.starts_with("test_") || name.starts_with("Test")` for PowerShell (crates/cgg-resolve/src/deadcode/roots.rs:975-988).
  - Every function using the approved PowerShell verb `Test-` (`Test-Path`-style, `Test-TargetResource`, `Test-Thing`) is a `builtin:test-name` **test root** (roots.rs:793-801).
  - f7: `Test-TargetResource test-entry builtin:test-name`.
  - These names are also counted into `coverage_test` (roots.rs:718-723).
- **`*.Tests.ps1` file gating**: no PowerShell-specific test-file recognition. Unverified whether the generic test-path heuristic covers `*.Tests.ps1` (not found by grep in roots.rs).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: PowerShell is not listed (rules.rs:11806, `["python","php","ruby"]`).
- **Benchmark / fuzz harnesses**: none.

## 12. Dead-code analysis

- **Class `script-driven`** (caps.rs:27-36, 165-166). For root-dependent findings, every finding gets evidence `language-is-script-driven` → confidence **Low** (cgg-core/src/deadcode.rs:314-316).
  - With the default threshold all are withheld. f6: `0 … marked unreferenced at high confidence, 5 withheld`.
  - With `--dead-code-confidence low`, 5 are shown, each with risk `language-is-script-driven, language-lacks-attributes, language-lacks-dispatch-model, language-lacks-value-references, language-lacks-visibility` (f6 `out.json.deadcode.txt`).
- **Capability row** (f6 deadcode JSON): visibility/attributes/value_references/dispatch/exports/test_tagging all `none`, `max_confidence: medium`, `root_rules_active: ["toplevel:invocation"]`.
- **Roots**:
  - **`toplevel:invocation`**: a function called from script top level (roots.rs:590-620). f6: `Public-Fn` rooted by `Public-Fn` on line 8, which makes `Private-Fn` reachable. f7: `Build-Body` in run.ps1, `Test-Helper` in a Pester file.
  - **`framework:entry-node`**: DSC `*-TargetResource` (f7).
  - **`builtin:test-name`**: every `Test*` name (11).
- **Not roots**:
  - `Export-ModuleMember -Function X` and `FunctionsToExport` (f6: `Unused-Fn` etc. reported, and `Public-Fn` live only because of the top-level call).
  - `[CmdletBinding()]` functions (f7 `Get-Public` reported).
  - Class methods (f6 `Thing.Used`/`Thing.Unused` reported).
- **False findings from extraction loss**: `function Get-TargetResource { param($Name) Helper-Dsc }` drops its call (01), so `Helper-Dsc` is reported dead (f6, f7).
- **Caps**: `implements-external-contract`, `macro-generated` and `reflection` never apply (no overrides, macros or dynuse).
- **Visibility**: none recorded (`hidden` is ignored).
- **Unresolved-doubt handling**: generic. Evidence `no-unresolved-site-with-name` is checked case-sensitively, so a `get-foo` call site would not count as doubt for `Get-Foo`. This follows from 03; there is no separate fixture.

## 13. Builtins & stdlib tables

- **No `stdlib/powershell.txt`.** `stdlib_names("powershell")` returns `None` (cgg-core/src/stdlib.rs:184-257; the directory listing has no powershell file).
  - Built-in cmdlets (`Write-Host`, `ForEach-Object`, `New-Object`, `Out-Null`, …) all land in the **external** bucket, not stdlib. f5 `--include-external --include-stdlib` mints `<external>::New-Object`, `<external>::ForEach-Object` and `<external>::Get-Thing` (an undefined function) alike, with 0 stdlib.
  - The step-1d stdlib screen (cross_file.rs:6021-6022) therefore never excludes a PowerShell builtin name.
- **`rust_types`-like tables, `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, implicit packages**: none for PowerShell. `[T]::new` is not a recognised construction.
- **`--include-stdlib`**: produces nothing for PowerShell (f5 shows 0 stdlib).

## 14. Confidence & certainty

- **High** comes from:
  - intra-file unique-name binding (f1, f5, f8, f12);
  - type-literal static calls (f5 `[Service]::Create()`, f9 cross-file via dot-source);
  - **dot-source / Import-Module through `direct_imports`**: f3 `Main -> Get-Help2 high`, `Main2 -> Get-Private high`.
- **False highs observed**:
  - `([B]$this).Hello()` → self-edge high (bis2/c.ps1).
  - `[Service]::Create().Stop()` outer call bound high by bare name (f5, correct only by luck).
  - `Main2 -> Get-Private high`: binds a non-exported module function.
- **Medium**:
  - every untyped-receiver member call (duck fan-out, f5/f11);
  - every step-1d workspace-name binding (f2, f4).
- **Low**: framework entry edges (f7).
- **Mermaid `?` marker**: shown on every medium arrow (f5 `--include-external` mermaid: `N4 -->|?| N0`).
- `CertainCtx` / `OwnerFacts` typed-receiver certainty cannot apply, because no receiver type is ever recorded (05).

## 15. Dynamic use & reflection

- **`dynuse`**: PowerShell is not covered (plugins/dynuse.rs has no `powershell` string; the `dyn_uses` signal is false).
- `Invoke-Expression "…"`, `& $fn` with `$fn = "Name"`, `Get-Command Name`, `$obj.$m()`, `$obj.PSObject.Methods`, `[scriptblock]::Create("…")` and `Invoke-Command -ScriptBlock ${function:X}` are all unmodelled.
  - `$x.$m()` becomes a literal external `$m` (f9 L13).
  - `Invoke-Expression` is an external command (f1 L43).
- **Module loads**: `Import-Module` with a variable path (`Import-Module $modPath`) records `$modPath` literally and resolves nothing. The pester gap notes this pattern in PowerShellGet (rules.rs:6479).

## 16. Known gaps & limitations

1. **Shebang misrouting**: `pwsh` / `powershell` shebangs are claimed by Bash's `sh` needle, even on `.ps1` files (f9; detect.rs:63-75).
2. **Script top level is not a callable**: every top-level call is unattributed (`no-enclosing-callable`). This covers scripts, Azure Functions `run.ps1` and Pester files; see the `azure-functions` gap text (rules.rs:4633).
3. **Script blocks are not callables**: Pester `Describe`/`It`, `-ScriptBlock`, `$sb = {}` (rules.rs:6479).
4. **Grammar parse failures lose calls**: an inline `param(...) Call` drops the call silently (f8). Constructor `: base()`, `./script.ps1` invocation and DSC `configuration` also put the file in error (01).
5. **Case-sensitive name matching** in a case-insensitive language (f12).
6. **No receiver typing** from parameter types, typed locals, `[T]::new()` or `New-Object` (f5).
7. **No base-class records or inheritance dispatch**, even under `--dynamic-dispatch` (f11).
8. **Constructors are never call targets**: `[T]::new()` is external (f1, f9).
9. **Backslash import paths do not resolve** on POSIX, and dot-source/Import-Module is non-transitive (cross_file.rs:3491-3504).
10. **Any import enables workspace-wide bare-name binding** (step 1d), binding all ≤3 copies at medium (f4).
11. **Exports / visibility / attributes are not recorded**: `Export-ModuleMember`, `FunctionsToExport`, `[CmdletBinding()]`, `hidden` (rules.rs:6532 gap).
12. **`Test-*` verb functions are misclassified as tests** (roots.rs:987).
13. **No stdlib table**: builtin cmdlets are indistinguishable from unknown externals (13).
14. **Aliases, module-qualified `Mod\Fn`, scope modifiers (`global:`), quoted/variable `&` targets and `${function:X}`** are unresolved or unrecorded (03, f14).
15. **Nested functions are not lexically scoped**, and redefinition / conditional definitions are ambiguous (f10).
16. **Enums, properties and DSC configurations are not extracted** (f1, f14).
17. **Grammar quirk**: a `1..3 |` range is recorded as a command (f1, f9).

The CHANGELOG has PowerShell entries only for the computed `-ChildPath` import (CHANGELOG.md:1929) and the DSC detection gap (CHANGELOG.md:2201). No `#[ignore]` PowerShell tests exist (powershell.rs:401-548; `crates/cgg/tests/detect_prefixes.rs:72` `every_detect_prefix_can_actually_fire` covers the detect prefixes via `Import-Module {prefix}`).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.ps1/.psm1/.psd1` work case-insensitively; pwsh shebangs go to bash; several constructs break the parse and lose calls |
| 02. Callables extracted | partial | functions, filters, workflows, class methods/ctors/statics; no top level, script blocks, enums, properties or DSC configs; flat nested names |
| 03. Call-site forms | partial | commands, `&` with a bare name, `$x.M()`, `[T]::M()`; no quoted/variable `&`, script paths, `${function:}`, aliases; case-sensitive matching |
| 04. Imports & module resolution | partial | Import-Module / dot-source / using recorded; forward-slash paths resolve high; backslash paths fail; non-transitive; exports ignored; any import enables a medium global fallback |
| 05. Type inference & receiver typing | minimal | only type-literal receivers; variables, params, casts and `[T]::new()` untyped |
| 06. Inheritance, dispatch & overloading | none | no bases, no override linking, not in INHERITANCE_LANGS or OVERLOAD_ARITY_LANGS |
| 07. Scoping & shadowing | none | no locals, no lexical narrowing; redefinitions and conditional defs ambiguous |
| 08. Macros, preprocessor & codegen | n/a | no macros; aliases and Invoke-Expression unexpanded |
| 09. FFI & cross-language links | none | no Add-Type/C# cmdlet bridge; script invocation not linked |
| 10. Frameworks & entry points | minimal | 4 rules; only powershell-dsc mints entries (tree-wide activation); module/pester/azure-functions detect-only |
| 11. Test harness entries | none | Pester detect-only; `Test-*` verb functions falsely rooted as tests |
| 12. Dead-code analysis | minimal | script-driven: all findings low and withheld; only toplevel:invocation and DSC roots; exports not roots |
| 13. Builtins & stdlib tables | none | no stdlib/powershell.txt; cmdlets land in external |
| 14. Confidence & certainty | partial | high for intra-file and include-anchored bindings; false highs from cast/chained receivers and non-exported module functions |
| 15. Dynamic use & reflection | none | no dynuse; Invoke-Expression, `& $fn`, `$x.$m()` unmodelled |
| 16. Known gaps & limitations | n/a | 17 gaps listed; shebang misrouting, top-level/script-block blindness and silent call loss are the most severe |
