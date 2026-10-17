# bash

The Bash plugin (`BashPlugin`, `crates/cgg-lang/src/plugins/bash.rs`, 291 lines including 4 inline tests) parses shell scripts with the crates.io grammar `tree-sitter-bash` (workspace pin `"0.25"` at Cargo.toml:95, resolved to 0.25.1 in Cargo.lock:1418-1419; `tree_sitter_bash::LANGUAGE`, bash.rs:21-23). Nothing is vendored. A single recursive `BashWalker` (bash.rs:43-212) does three things:

- It records every `function_definition` as a `FreeFunction` named by its bare name.
- It records every `command` whose name is not in a hard-coded builtin skip list as a bare `RefRecord` with no receiver.
- It records `source FILE` / `. FILE` as an `ImportRecord` of kind `"source"`.

The plugin does not override `signals()`, so every optional signal is false (lib.rs:304-306): no visibility, attributes, exports, test_defs, value_refs, dyn_uses, unreachable or impls. It also does not override `resolver_kind`, `max_line_bytes` or `ts_language_for`, and it ignores `ExtractCtx` (`_ctx`, bash.rs:27). Resolution uses the generic pipeline. The one Bash-specific resolver branch is the `"source"` import arm in `cross_file.rs:1829-1844`, which treats a sourced file like a C `#include` (budget 4) through `collect_include_defs`.

Fixtures cited below live under `<scratch>/parity/bash/` (f1 … f12). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`. The results were dumped with `show.py` (graph) and `aud.py` (audit sidecar) in the same directory.

## 01. Detection & parsing

- **Extensions `.sh` and `.bash`** (bash.rs:15-17). Evidence:
  - f2: `lib/a.sh` reports `detected_via: extension:.sh`.
  - f7: `x.bash` reports `extension:.bash`.
- **Shebang needles `bash`, `sh`, `zsh`** (bash.rs:18-20). Detection runs `word.contains(needle)` over the whole first line, iterating plugins in registration order (detect.rs:63-75, `read_shebang` detect.rs:209-223). Bash is registered 12th (plugins.rs:135). Fixtures:
  - f1: `#!/bin/bash` → `shebang:bash`.
  - f2: `#!/bin/sh` → `shebang:sh`.
  - f7: `#!/bin/dash`, `#!/bin/ksh` and `#!/bin/zsh` all → `shebang:sh`. The `zsh` needle can never win, because `sh` is checked first and is a substring of `zsh`.
- **Shebang beats extension, and the `sh` substring over-matches** (partial / bug). Rule 1 (shebang) runs before Rule 2 (extension), so these files are routed to Bash (f7):
  - `#!/usr/bin/env pwsh` in `ps.ps1` → bash. PowerShell is registered after Bash (plugins.rs:155), and `pwsh` contains `sh`.
  - `#!/home/shared/bin/perl` in `perlscript.pl` → bash (`shared` contains `sh`).
  - `#!/usr/bin/env fish` → bash, with `parse_status: error` and 0 callables.

  Interpreters registered *before* Bash are safe: `#!/usr/bin/env python3` in `py.sh` → python (f7).
- **Not detected** (all f7, `file_skipped … unknown-extension`):
  - `.zsh`, `.ksh` and `.bats` extensions;
  - dotfiles such as `.bashrc`;
  - extensionless `#!/usr/bin/env bats`.
- **Content detection**: none for Bash. The only content sniff is the YAML/JSON descriptor sniff (detect.rs:77-97).
- **Grammar**: one grammar, no dialects. POSIX `sh`, `dash`, `ksh` and `zsh` scripts are all parsed with the Bash grammar. The f7 ksh/dash/zsh fixtures parsed `ok` because they used Bash-compatible syntax; zsh-only syntax was not tested (unverified).
- **Re-parse/fallback**: none. A file with syntax errors is extracted as-is with `parse_status = "error"` (f7 `fishscript`).
- **Depth skip**: `MAX_TREE_DEPTH` = 4000 and `SkipReason::TooDeep` are shared by every language (cgg/src/lib.rs, per CLAUDE.md). There is no Bash-specific handling.
- **Minified / long-line handling**: `max_line_bytes` is not overridden (default `None`, lib.rs:313-315). There is no locals spec, so the 2,000-byte locals rule has nothing to skip.
- **Heredoc bodies**: a `$(…)` inside an unquoted heredoc body is walked and recorded. f8: `cat <<EOT\n$(plat)\nEOT` produces edge `go -> plat L12`. The `heredoc_body` is a sibling of the command in the tree, not one of its arguments.

## 02. Callables extracted

Every definition is `DefVariant::FreeFunction`, with `simple_name == qualified_name ==` the literal function name (bash.rs:87-112). Other properties of every definition:

- There is **no owner, no module, no file and no directory component** in the qualified name.
- `visibility` is the empty string and `attributes` is empty.
- `signature_hint` is `extract_signature(text)` (plugins.rs:74), which yields `name()` and never parameters.

Supported shapes:

- **POSIX form** `plain() { …; }` → `plain` (f1 L2). Test `function_definitions` (bash.rs:234-245).
- **`function` keyword without parens** `function kw { …; }` → `kw` (f1 L3).
- **`function` keyword with parens** `function kwparen() { …; }` → `kwparen` (f1 L4).
- **Subshell body** `sub() ( … )` → `sub` (f1 L5).
- **Names with punctuation**: `my-func()`, `ns::fn()` and `dot.fn()` are each one callable named exactly so (f1 L6-8). `ns::fn` is **not** split into an owner `ns`.
- **Nested definitions**: `outer() { inner() { …; }; inner; }` gives two flat nodes, `outer` and `inner` (f1 L9-10). `walk_children` continues into the body (bash.rs:55-59). The inner node is not qualified by its parent, and calls inside `inner` are attributed to `inner` (f1 edge `inner -> plain`).
- **Redefinitions** `dup() {…}; dup() {…}` give two nodes with identical QNs (f1 L13-14, f2).
- **Conditional definitions** `if …; then plat() {…}; else plat() {…}; fi` give two nodes, both kept (f8).

Not supported:

- **Aliases** `alias ll='ls'` define no callable. A later `ll` is recorded as an external call (f8 `<external>::ll`).
- **Dynamically defined functions** (`eval "f_$x() { …; }"`) define nothing. The `eval` command is skipped entirely (bash.rs:184).
- **Scripts as callables**: a script invoked as a program (`./other.sh`, `bash other.sh`) is not a callable. There is no file-level node, and top-level code is not a callable (see §03 and §12).
- **Abstract members, constructors, properties, operators, overloads, closures**: not applicable to the language.

## 03. Call-site forms

Each recorded site is a `RefRecord` holding `name`, an empty `receiver_hint` and the line/byte (bash.rs:200-206). No arity, receiver, `path_receiver`, `value_ref` or `paren_less` flag is ever set. When a `command` is matched, the walker **returns without visiting the command's children** (bash.rs:60-64). A `declaration_command` also returns without visiting children (bash.rs:65-69, `check_source` is an empty stub at bash.rs:209-211).

Recorded (f1 unless noted):

| Shape | Example | Result |
| --- | --- | --- |
| Bare command | `plain` | edge `callers -> plain L16` |
| Command with arguments | `kw arg1 arg2` | edge L17 |
| Pipeline | `plain \| kwparen` | both recorded (L18) |
| `&&` / `\|\|` lists | `plain && sub \|\| my-func` | all three recorded (L19) |
| Command substitution in a plain assignment | `x=$(ns::fn)` | recorded (L20) |
| Redirected statement | `plain > /dev/null 2>&1` | recorded (L25) |
| Subshell / group | `( inner )`, `{ kw; }` | recorded (L34, L35) |
| Loop and condition bodies | `if kw; then sub; fi`, `while plain; do`, `for …; do kwparen; done` | recorded (L24, L36, L37) |
| `case` arm body | `start) plain ;;` | recorded (L38) |
| `$(…)` inside `[[ … ]]` | `[[ -n "$(kw)" ]]` | recorded (L39) |
| `$(…)` inside an array assignment | `arr=( $(sub) )` | recorded (L40) |
| `$(…)` inside a heredoc body | see §01 | recorded (f8 L12) |

Recorded but with an unusable name:

- **Backtick substitution** `` `plain` `` is recorded as one name including the backticks, `` `plain` ``. It is classified `external`, so the edge is lost (f1 L41, audit `external_calls`).
- **Dynamic command names** are recorded verbatim as the name and classified `external` (f1 L26-27):
  - `"$CMD" arg` → name `"$CMD"`;
  - `$CMD arg` → name `$CMD`.
- **Path invocation** `./other.sh` is recorded as name `./other.sh` and classified `external` (f1 L28). It is never linked to the script file.

Not recorded (lost edges):

- **Calls inside a command's arguments.** Children of a `command` node are never walked (bash.rs:60-64).
  - `echo "$(plain)"` loses `plain` (f1 L23).
  - The same holds for any `cmd "$(f)"` and any process substitution passed as an argument (`diff <(f) <(g)`), by the same code path. The process-substitution case was not run (unverified).
- **Calls inside a declaration's value.** `declaration_command` is not walked (bash.rs:65-69).
  - `local y=$(dot.fn)` loses `dot.fn` (f1 L21).
  - `export Z=$(dup)` loses `dup` (f1 L22).
  - `declare`, `readonly` and `typeset` follow the same node kind (unverified per keyword).
- **Builtin skip list.** These names are dropped before any record is made, so they appear in no audit bucket (bash.rs:164-198): `echo printf cd exit return export local readonly unset set shift test [ [[ true false : eval exec trap wait kill read declare typeset let pushd popd dirs`. f8's `--include-stdlib` graph has no `<stdlib>::echo`.
- **Function names passed as arguments.** Command wrappers are recorded as the wrapper only; the wrapped function name is an argument and is lost. Examples: `sudo plain`, `command kwparen`, `xargs -n1 plain`, `time plain`. The wrappers land in `stdlib_calls` (f1 L29, L30, L42, L43).
- **`trap 'handler' EXIT` / `trap handler EXIT`.** `trap` is in the skip list, so the handler is never referenced (f1 L31, f11 `on_exit`).
- **`eval "plain"`**: skipped (f1 L32).
- **Not applicable to the language**: method calls, qualified/path calls, constructors, explicit generics, call operators, method references and registrar arguments. `ExtractCtx` registrar verbs are ignored (bash.rs:27).
- **Arity**: never recorded.

## 04. Imports & module resolution

- **`source FILE` and `. FILE`** become `ImportRecord { kind: "source", path, alias: "" }` (bash.rs:130-162). The path is the command's first `argument` field, or else the first `word`/`string` child, with surrounding `"`/`'` trimmed. Test `source_directive_is_import` (bash.rs:263-281).
- **Resolution** happens in the `"source"` arm (cross_file.rs:1829-1844) → `collect_include_defs(path, …, depth 4)` (cross_file.rs:3385-3523). That function:
  1. joins the path onto the includer's directory (lexically normalising `./` and `../`, cross_file.rs:3420-3438);
  2. tries an exact path match;
  3. otherwise tries a suffix match among files with the same last segment (cross_file.rs:3457-3484);
  4. pushes every definition of the target file into `direct_imports` (cross_file.rs:3496-3501).

  Evidence:
  - `. lib/a.sh` resolves, giving `main -> a_fn` **high**, `cross-file:imports` (f2, f5a).
  - `source b.sh` (bare relative name) and `source ../lib/c.sh` (parent-relative) both resolve (f12).
  - Integration test `bash_source_resolves` (crates/cgg/tests/resolve.rs:1500-1543).
  - CHANGELOG ~1944-1952 notes that `..` normalisation also serves Bash `source`.
- **Partial: sourcing is not transitive.** The recursive follow in `collect_include_defs` only descends into imports of kind `include`/`system-include` (cross_file.rs:3507-3521), never `source`. The `depth 4` budget is therefore never consumed past one level. In f3, `main.sh` sources `lib/a.sh`, which sources `./c.sh`. A call to `c_fn` is not scoped to `lib/c.sh`: it falls to the global fallback and binds **both** `lib/c.sh` and `other/c2.sh` at medium.
- **Bug: a sourced name binds to an arbitrary copy at `high`.** Step 1 looks up each `direct_imports` qualified name through `by_qn` (cross_file.rs:5710-5755). Only the C family gets `c_include_copies` scoping (cross_file.rs:5711-5744). Bash QNs are bare names, so `by_qn` returns whichever same-named callable it kept, not the one in the sourced file. Evidence:
  - f10: `source ./lib/l.sh` plus `shared` → edge to **`zz/decoy.sh`**, high.
  - f12: `source b.sh`, which resolves exactly to `lib/b.sh`, still binds `x/b.sh` at high. `source ../lib/c.sh` likewise binds `x/c.sh` at high.
  - f8/f9: two sourced copies bind to one (`lib/l2.sh`) at high regardless of `source` order.
- **Variable paths are not resolved.** Paths such as `source "$HOME/lib/x.sh"`, `source "${DIR}/lib/dup1.sh"` and `source "$(dirname "$0")/a.sh"` keep their `$…` text, so neither the exact nor the suffix match fires (f2, f3, f12). The `bash-dispatch` gap text (rules.rs:6557) notes this is the common real-world form.
- **Gate for global by-name fallback.** Step 1d (cross_file.rs:6015-6050) binds a bare name to every same-named callable in any file, at medium, but only when all of these hold:
  - the calling file has at least one *resolved* import (`has_imports = !direct_imports.is_empty() || …`, cross_file.rs:6018);
  - there are ≤3 candidates;
  - the name is not in `stdlib/bash.txt`.

  Consequences:
  - A file whose `source` resolved gets by-name binding for **every** bare call, even to files it never sourced. f5a: `lone_fn` gets medium.
  - A file with no resolvable `source` gets **no** cross-file edges at all. f4 and f5b/f5c: every cross-file call is `candidates-in-other-files`, including a unique name.
  - f3: `many` with 7 candidates stays unresolved (`candidates-in-other-files {candidates: 7}`) in a file without a resolving source.
  - f2/f3: an unresolvable variable source path still yields both copies of `twin` at medium.
- **Not used for Bash**: `FileFacts::packages`, manifest reading, `heads.rs` external-module refusal (`WorkspaceVocab` covers Python/JS/TS/Go/Java/Kotlin/C#/C++ only, per CLAUDE.md), `modules.rs` `not-in-imported-module`, re-exports and aliases. A grep for `"bash"` in `crates/cgg-resolve/src/` finds only `type_hints.rs:348` (perf tally) and `deadcode/caps.rs:28`.
- **External classification**: a bare command name with no candidate anywhere and not in `stdlib/bash.txt` is recorded in `external_calls` with `no-candidate-in-file`. Examples: `curl` and `git` (f6), `missing_fn` (f2), `main` (f1 L48).

## 05. Type inference & receiver typing

Not applicable: Bash has no types, receivers or method calls. No Bash branch exists in `type_hints.rs`. The only `"bash"` occurrence there (type_hints.rs:348) is a row in `lang_tallies_table!`, a per-language perf counter for `--metrics`, not inference. No `locals`, `local_types`, `scoped_types`, `typed_receiver`, `weak_receiver_type` or `implicit_receiver` are produced (bash.rs: none of these fields are set). Bash is absent from `CAST_INTRINSICS` (stdlib.rs:35) and `CONSTRUCTING_METHODS` (stdlib.rs:58, Ruby only).

## 06. Inheritance, dispatch & overloading

Not applicable: Bash has no classes, interfaces, overloads or `super`. Bash is absent from:

- `INHERITANCE_LANGS` (dispatch.rs:110-123);
- `OVERLOAD_ARITY_LANGS` (no arity recorded, §03);
- `member_of_calling_class`.

`--dynamic-dispatch` produces nothing for Bash. f11 `--dead-code` capabilities: `dispatch: none`.

A related shape is redefinition (`f(){…}; f(){…}`). At runtime the last definition wins. cgg binds **both** copies at medium: f2 `main -> dup` ×2 is medium with resolver `cross-file:imports`, even though both are in the caller's own file. There is no "last definition wins" rule.

## 07. Scoping & shadowing

- **No `FileFacts::locals`**: `locals.rs` has no Bash spec (grep `"bash"` in `crates/cgg-lang/src/*.rs` finds nothing). As a result, `shadowed-by-local` never fires for Bash.
- **No lexical narrowing**: Bash is not in the lexical-narrowing list (JS/TS, Lua, OCaml, F#, Scala, Kotlin, Swift, Rust, Dart, Haskell, Julia, per CLAUDE.md). Nested functions are flat (§02). An inner function defined in `outer` is visible to every caller in the file and, at runtime, only exists after `outer` has run. cgg binds `callers -> inner` high (f1 L34).
- **Conditional definitions**: there is no `DefRecord::conditional` (PHP only). Both branches bind at medium. f8 `go -> plat` is `?2x` in mermaid.
- **Intra-file priority**: a same-file definition wins over a sourced one. f6: `cp` defined in-file binds intra-file high. `install`/`log` from a sourced file bind high (cross-file:imports) even though both are in `stdlib/bash.txt`, because the stdlib screen applies only to step 1d (cross_file.rs:6022-6024).
- **Positional semantics are ignored**: a call that precedes the definition in source order still binds. Not specifically tested; this follows from the span-free name lookup (unverified as a fixture).

## 08. Macros, preprocessor & codegen

Not applicable: Bash has no macros or preprocessor. The plugin sets no `fn_macros`, `macro_invocations`, `name_mentions` or `macro:<name>` tags (bash.rs). Aliases (`alias x='…'`) are the nearest analogue and are not expanded (§02, f8).

## 09. FFI & cross-language links

None. `ffi.rs`, `descriptor.rs` and `markup.rs` have no Bash branch. A grep for `bash` finds nothing in `crates/cgg-resolve/src/ffi.rs`, `descriptor.rs` or `crates/cgg-lang/src/plugins/markup.rs`.

Bash scripts are the archetypal glue that invokes other programs: `python tool.py`, `node x.js`, `./build.sh`, `make target`. cgg links none of these to the invoked file or entry point. The program name is recorded as an external call (f1 `./other.sh`, f6 `git`/`curl`), and the script argument is dropped (§03).

## 10. Frameworks & entry points

One rule names Bash. No multi-language rule includes it: there is no `languages: &[` or `"*"` in rules.rs.

- **`bash-dispatch`** (rules.rs:6536-6560)
  - Kind: `TrustKind::Cli`, `node: true`.
  - Trigger: a `source` import whose path is one of `/etc/os-release`, `/etc/profile`, `/etc/bashrc`, `/etc/init.d/functions`, `/etc/rc.d/init.d/functions`, `/lib/lsb/init-functions` or `/usr/share/bash-completion/bash_completion`.
  - Empty fields: `detect_paths`, `detect_calls`, `attributes`, `registrars`, `base_types` and `methods` are all `NONE`.
  - Effect: detection only. It reports as `seen_no_rules` with its `gap` text and mints **no** entry nodes. f6: `. /etc/os-release` → `framework_coverage.seen_no_rules = [bash-dispatch]`, `nodes_minted: 0`.
  - The gap text names four entry mechanisms cgg cannot see:
    1. `case "$1" in … ) fn "$@" ;;` argv dispatch;
    2. dynamically built names (`eval "$m"`, `"dns_${p}_add"`);
    3. `trap … EXIT` handlers;
    4. `main "$@"`.

    The text also notes that real `source` targets are variables, so the detect rarely fires (e.g. acme.sh).
- **Other rule tables**: `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for` and visibility entries all have no Bash entries. Bash has no attributes for them to read.
- **Registrar verbs**: none. `ExtractCtx` is unused (bash.rs:27).
- **Interaction with dead code**: top-level invocations are the de-facto entry roots (§12). README line 1195 shows Bash listed under "no rules".

## 11. Test harness entries

None.

- There are no `kind: test` rules for bats, shunit2 or shellspec (grep `bats|shunit|shellspec` in rules.rs finds nothing).
- `.bats` files are not even detected (§01, f7).
- No `TestRole` is assigned: `test_defs` is false and there is no `go_test_role` analogue.
- Bash is not in `TEST_CLASS_MEMBERS_ARE_ROOTS`.
- **Test-file gating**: `classify_test_file` has no Bash filename convention (testfile.rs:76-119; for example `*_test.sh` is not recognised). Only the generic directory rule applies: a path under `test/`, `tests/`, `__tests__/` … (`TEST_DIRS`, testfile.rs:36, applied at testfile.rs:125-131).

## 12. Dead-code analysis

- **Class**: Bash is in `SCRIPT_DRIVEN` (deadcode/caps.rs:27-36), so `LanguageClass::ScriptDriven` (caps.rs:165-166). Test `script_languages_are_classified_apart` (caps.rs:250-256).
- **Confidence cap**: `Evidence::LanguageIsScriptDriven` caps every finding at **low** (cgg-core/src/deadcode.rs:314-316). Root-dependent categories are additionally suppressed with `SuppressionReason::ScriptDriven` (deadcode/mod.rs:811-812). Every Bash finding is therefore withheld by default.
  - f6/f11: `findings 0 shown (high) · N withheld (low)`.
  - Evidence list on each finding: `language-is-script-driven`, `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references`, `language-lacks-visibility`, `no-incoming-edges`.
  - The capabilities row reports `max_confidence: "medium"`, but the findings themselves come out `low` (f11).
- **Roots**: only `toplevel:invocation` (roots.rs:555-620). A top-level call site recorded as `no-enclosing-callable` roots **every** Bash callable sharing that simple name, across all files.
  - f11: `main "$@"` → root `main`.
  - f6: `run` inside a top-level `case` → root `run`.
  - f1 shows each top-level site recorded twice in the audit, as `no-enclosing-callable` and `candidates-in-other-files`.
  - f1 L48: a top-level call to a name with no definition is classified `external` instead.
- **Signals**: visibility, attributes, value refs, dispatch, exports and test tagging are all `none` (f11 capabilities). There is no `unreachable` signal (no `signals()` override).
- **Missed liveness**, all reported as never-referenced (f11):
  - `trap handler EXIT` (`on_exit`);
  - dynamic dispatch `"cmd_$1"` (`cmd_x`; the site is recorded as an external named `"cmd_$1"`, and nothing matches it by prefix);
  - functions only called inside `$(…)` arguments or `local x=$(…)` (§03).
- **No `BLIND_SPOTS` entry for Bash** (caps.rs:44 onward lists rust, python, javascript, typescript, kivy …). The report therefore does not name the lost-argument and `trap` blind spots.
- **No dynuse doubt**: `eval` and dynamic names do not cap same-named findings, because dynuse covers only Python/JS/Java/C#/Ruby/PHP.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/bash.txt`**: 170 lines, loaded at stdlib.rs:194 and 235. Contents:
  - shell builtins (`. : [ alias bg bind break builtin caller cd command compgen complete … wait`);
  - common coreutils/tools (`awk cat cp cut date find grep gzip head install journalctl ls mktemp openssl ps sed sort sudo systemctl tail tar tee timeout tr uniq xargs xxd yq zip` …).
- **Effect**:
  - Matching names go to the `stdlib_calls` audit bucket (f1 `sudo command grep xargs time`).
  - The step-1d global fallback never binds them (cross_file.rs:6022-6024).
  - `--include-stdlib` mints `<stdlib>::name` leaves (f8 `<stdlib>::cat`, `grep`, `alias`).
- **Second, separate builtin filter**: the plugin's hard-coded skip list (bash.rs:164-198, §03) drops 29 names before resolution. Those never appear in any bucket, even with `--include-stdlib`. The two lists overlap but are maintained separately.
- **Not in the stdlib list**, so classified **external**: `curl`, `git`, `docker`, `jq`, `make`, `python`, `bash`, `wget` (f6 `curl`/`git` → `external_calls`; `--include-external` gives `<external>::curl`, f8).
- **Tuning history**: CHANGELOG ~5971 lists bash among the 21 stdlib lists tuned against corpus external-bucket noise.
- **Not applicable**: `rust_types` tables, `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, implicit packages.
- **User functions with stdlib names still bind**:
  - in-file `cp()` → intra-file high (f6);
  - sourced `install()`/`log()` → high (f6).

## 14. Confidence & certainty

| Path | Confidence | Evidence |
| --- | --- | --- |
| Intra-file unique name | high | f1, every `intra-file` edge |
| Sourced-file name via `direct_imports` | high | f2 `a_fn`, f6 `install` |
| Same name defined twice | medium (all copies) | f2 `dup`, f8 `plat` |
| Step-1d global by-name fallback | medium | f5a `lone_fn`, f3 `c_fn`, f12 `a1` |

- **Sourced-file high is not structurally certain.** The `by_qn` collision (§04) makes it a false high when another file defines the same name: f10 binds a decoy at high, and f12 binds the wrong `b.sh`/`c.sh` at high. CLAUDE.md's `CertainCtx` rule ("one callable carries the name: no vendored copies") is evidently not enforced on this path for Bash.
- **Mermaid** prefixes a label with `?` when the arrow has no high site. f8: `N4 -->|?2x| N2`.
- **Never produced**: `low` (`--guess-unique-names` is Rust-only) and `Via::Reference` edges (no value refs).

## 15. Dynamic use & reflection

None. Bash is not covered by dynuse (Python/JS/Java/C#/Ruby/PHP only, per CLAUDE.md), and `dyn_uses` is false. In detail:

- `eval …` is dropped by the skip list (bash.rs:184).
- `"$fn" args`, `$fn args` and `"prefix_$1"` are recorded as literal external names (f1 L26-27, f11). They create no doubt, no reflection screen and no prefix match.
- `declare -f`, `type`, `command -v`, `compgen -A function`, `${!var}` indirection and `trap` strings are not modelled.

## 16. Known gaps & limitations

1. **False-high binding of sourced functions** to an arbitrary same-named copy in an unrelated file (`by_qn` collision; only the C family gets `c_include_copies`, cross_file.rs:5711-5744). Fixtures f10 and f12. Most severe.
2. **Shebang detection misroutes other languages to Bash.** The `sh` substring match runs before the extension rule (detect.rs:63-75, bash.rs:19). Affected: `pwsh` scripts (even `.ps1`), any interpreter path containing `sh` (`/home/shared/…/perl`), and `fish`/`csh`/`tcsh`. The `zsh` needle is dead (f7).
3. **Calls inside command arguments and declaration values are lost**: `echo "$(f)"`, `local x=$(f)`, `export X=$(f)`, `sudo f`, `xargs f`, `time f`, `trap f EXIT` (bash.rs:60-69; f1 L21-23, L29-31, L42-43).
4. **`source` is not transitive** (cross_file.rs:3507-3521 follows only `include`/`system-include`; f3). Variable paths (`"$DIR/x.sh"`, `"$(dirname "$0")/x.sh"`) never resolve (f2, f12).
5. **The cross-file gate is all-or-nothing.** A file with no resolved `source` gets zero cross-file edges, even to a unique name (f4, f5b, f5c). A file with one resolved `source` gets by-name edges to anything with ≤3 copies (f5a; cross_file.rs:6018).
6. **Backtick substitution** is recorded with the backticks in the name, so the edge is lost (f1 L41).
7. **No `.zsh`, `.ksh` or `.bats` extensions**, no dotfile (`.bashrc`) detection, and no bats/shunit2 test harness rules (f7; §11).
8. **Entry mechanisms are invisible** (rules.rs:6557 gap text): argv `case` dispatch (partly covered only when at top level, via the `toplevel:invocation` root), `trap` handlers, `eval`/dynamic names, and script-to-script invocation.
9. **Dead code is suppressed wholesale**: script-driven, so every finding is low and withheld (deadcode.rs:314-316, mod.rs:811-812). There is no Bash `BLIND_SPOTS` entry.
10. **Redefinition and conditional definition** bind every copy at medium. There is no last-definition-wins rule (f2, f8).
11. **Nested functions are flat** and globally visible (f1).
12. **Two separate builtin lists drift independently**: the plugin skip list (bash.rs:164-198) and `stdlib/bash.txt`.
13. **`check_source`** is a dead stub (bash.rs:209-211). It is called for every `declaration_command`, which causes the early return that loses `$(…)` values.
14. **Test coverage is thin**: 4 inline tests (bash.rs:234-290), 1 integration test (`bash_source_resolves`, resolve.rs:1500) and 1 prefix-detection row (detect_prefixes.rs:73). There are no tests for shebang collisions, nested definitions, argument-position calls or decoy copies. No ignored tests.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.sh`/`.bash` and shebangs work, but the `sh` substring steals `pwsh`/perl paths/fish files; no `.zsh`/`.ksh`/`.bats`/dotfiles |
| 02. Callables extracted | partial | Every function form is captured; names are bare and flat (no owner, nesting or file), aliases are ignored |
| 03. Call-site forms | partial | Bare commands everywhere except inside command arguments/declarations; backticks are mangled; wrappers lose their target |
| 04. Imports & module resolution | partial | `source`/`.` with literal paths resolves, but is non-transitive, variable paths fail, and copies are false-high via `by_qn` |
| 05. Type inference & receiver typing | n/a | Untyped language; nothing recorded |
| 06. Inheritance, dispatch & overloading | n/a | No classes or overloads; redefinitions bind all copies at medium |
| 07. Scoping & shadowing | minimal | No locals or lexical narrowing; same-file definitions win over sourced ones |
| 08. Macros, preprocessor & codegen | n/a | No macros; aliases are not expanded |
| 09. FFI & cross-language links | none | Script-to-program/script invocations are not linked |
| 10. Frameworks & entry points | minimal | One detect-only rule (`bash-dispatch`) that mints no nodes |
| 11. Test harness entries | none | No bats/shunit2 rules, `.bats` is undetected, only the generic test-dir rule |
| 12. Dead-code analysis | minimal | Script-driven: every finding is low and withheld; only the `toplevel:invocation` root |
| 13. Builtins & stdlib tables | full | 170-name `bash.txt` plus a 29-name plugin skip list; `--include-stdlib` works |
| 14. Confidence & certainty | partial | High for intra-file and sourced names, but sourced high can be false when copies exist |
| 15. Dynamic use & reflection | none | `eval`, `"$fn"` and `${!x}` are not modelled; dynamic names become literal externals |
| 16. Known gaps & limitations | n/a | 14 gaps listed; false-high sourcing and shebang misrouting are the most severe |
