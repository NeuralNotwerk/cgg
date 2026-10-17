# 01. Detection & parsing

**Category definition.** This category covers everything between "a file exists in the walked tree" and "a tree-sitter tree is handed to the plugin's `extract`". That is:

- how a file is claimed by a language: extension, fixed filename, shebang, content sniff;
- how source embedded in a container format is pulled out: notebooks, sibling markup, templates, literate files;
- which grammar parses it, and whether a per-file dialect is chosen;
- what happens when the parse fails: re-parse, targeted repair, and how far damage spreads in the error-recovered tree;
- how conditional-compilation text is presented to the grammar;
- the guards that skip or degrade a file: tree depth, line length, minified bundles, walker deny-list, size, binary;
- the project-manifest reads done during extraction to give a file its module identity;
- the path-based test-file classification that runs on the detected language.

Resolution of imports is category 04, and macro *expansion* is category 08. Both appear here only where they change what is parsed.

Sources: section `## 01. Detection & parsing` and the Summary-scorecard row 01 of each `dev_docs/languages/<lang>.md`. The shared code was spot-checked: `crates/cgg-lang/src/detect.rs`, `crates/cgg/src/lib.rs`, `crates/cgg-walk/src/lib.rs`, `crates/cgg-core/src/testfile.rs` and `crates/cgg-lang/src/locals.rs`. New fixtures were run with `target/release/cgg` 0.11.0 under `scratchpad/parity/matrix-01/{t,u,v,w,x}`. Each is cited below as `fx:<dir>/<file>`.

## Shared machinery (applies to every row unless the row says otherwise)

| mechanism | behaviour | evidence |
| --- | --- | --- |
| Rule order | 1. shebang. 1b. `.yaml/.yml/.json` content sniff, which never falls through to the extension rule. 2. extension, matched case-sensitively first and then lowercased. 3. `.h` header verdict | `detect.rs:62-127` |
| Shebang match | `line.contains(needle)` over the **whole first line**, at most 256 bytes. Plugins are tried in registration order (`plugins.rs:124-169`) and the first hit wins. Python, JS and TS come before Bash (12th). Ruby, Lua, PHP, Scala, R, Julia, Perl, Elixir, Erlang, Haskell and PowerShell come after it, so any shebang line containing `sh` goes to Bash | `detect.rs:63-75`, `209-222`; fx:t/scl, fx:t/luas |
| Extension | `Path::extension()` takes only the last dot-segment. So `p.pkr.hcl` resolves as `.hcl`, and `x.tf.json` resolves as `.json` and is content-gated | `detect.rs:146-150`; fx:t/p.pkr.hcl |
| Case fallback | `X.PY` and `a.RS` are detected. A mixed-case entry (`.Rmd`) fails both passes for `d.RMD` | `detect.rs:101-127`; fx:t/X.PY, fx:t/d.RMD |
| Filenames | **No plugin can claim an extensionless filename.** There is no filename rule anywhere in `detect.rs` | `detect.rs:62-133` |
| Content sniff | Only two exist: `.h` → C or C++ (`header_verdict`, `has_cpp_content`), and YAML/JSON → OpenAPI or AsyncAPI (`sniff_structured_descriptor`) | `detect.rs:160`, `226`, `270` |
| Per-file grammar | Only TypeScript overrides `ts_language_for` | `typescript.rs:54-61`; a grep of `fn ts_language_for` finds no other plugin |
| Error tree | Extraction always runs on the error-recovered tree, and `parse_status` becomes `"error"`. There is no generic re-parse | `cgg/src/lib.rs:457-461` |
| Re-parse | C# only (`reparse_without_conditionals`) | `csharp.rs:683-698` |
| Depth guard | A tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep` | `cgg/src/lib.rs:155`, `451-456` |
| Line-length guard | Only when the plugin sets `max_line_bytes`, and only Kivy does (32 KiB). The skip reason is `SkipReason::LongLine` | `cgg/src/lib.rs:430-442`; `kivy.rs:85-87` |
| Parse timeout | **None** for any language. There is no `set_timeout`, progress callback or `ParseOptions` in the parser or the plugins | grep of `cgg-lang/src` |
| Minified, locals | `locals::collect` is skipped when any line is over 2,000 bytes. This only affects the 11 languages with a locals spec: js, ts, python, lua, scala, kotlin, swift, rust, dart, ocaml, zig | `cgg/src/lib.rs:159`, `478-486`; `locals.rs:380-392` |
| Minified, confidence | A file averaging over 1,000 bytes per line never yields a cross-file `high`. This is language-agnostic | `cross_file.rs:4545-4550` |
| Minified, walker | `--skip-minified` (off by default) applies to `js/mjs/cjs/css` only | `cgg-walk/src/lib.rs:77-81` |
| Notebook | The `.ipynb` → Python code-cell extractor runs only when the extension is exactly lowercase `ipynb` | `cgg/src/lib.rs:408-423`; fx:w/NB.IPYNB |
| Walker deny-list | Denied: `node_modules .venv venv site-packages vendor target build bin obj dist .git .gradle .cargo __pycache__ .next .nuxt`. Hidden directories are **walked** (`hidden(false)`) | `cgg-walk/src/lib.rs:49-66`, `195`; fx:x |
| Size and binary | Files over 25 MiB are skipped as `TooLarge`. A NUL byte in the first 8 KiB means `Binary` | `cgg-walk/src/lib.rs:143`, `314-348` |
| Test files | The `src/test/` layout counts for java, kotlin, scala and groovy. `tests/` and `benches/` count for rust. Name rules exist for go, python, js/ts, java, kotlin, csharp, ruby, php, swift, dart, elixir, erlang, c/cpp and scala. The generic `TEST_DIRS` are `test tests __tests__ spec specs testdata e2e Tests Test` | `testfile.rs:35-45`, `61-75`, `76-119` |

## Legend

| mark | meaning |
| --- | --- |
| ✅ | full: the capability exists and covers the ecosystem's common cases |
| 🟡 | partial: it exists but misses a common case, is hijacked, or loses data |
| ⬜ | none: the concept exists in the language, cgg does nothing |
| — | n/a: the concept does not exist or does not matter for this language |

### Columns

| column | capability |
| --- | --- |
| **Ext** | Extension coverage of the ecosystem's source files |
| **Names** | Fixed or extensionless filenames: `Gemfile`, `BUILD`, `CMakeLists.txt`, `Jenkinsfile`… |
| **Shebang** | Interpreter-line detection that actually wins |
| **Sniff** | Content detection, needed where an extension is shared or absent |
| **Embed** | Extraction of source embedded in a container: notebook, template, SFC, literate file, sibling markup |
| **Dialect** | Per-file grammar choice where the language has dialects or interface files |
| **Repair** | Re-parse or plugin-side repair of known grammar misparses |
| **CondComp** | How `#if`, build tags or reader conditionals reach the grammar |
| **ErrTree** | An error parse damages only the broken construct (✅), or loses later definitions or sites (🟡) |
| **Syntax** | The grammar accepts current legal syntax without `parse_status: error` |
| **Depth** | The `MAX_TREE_DEPTH` skip |
| **Time** | Pre-parse long-line cap or parse timeout |
| **Minified** | ✅ = all three minified layers (walker skip, locals gate, confidence); 🟡 = locals gate and confidence only; — = no locals spec and no minification practice (the confidence demotion still applies) |
| **Manifest** | A project manifest or file layout is read during extraction to give module identity |
| **TestFile** | A path-based test-file rule matching the ecosystem's convention |
| **DirFit** | The walker's built-in deny-list fits the ecosystem's layout: build output skipped, source dirs kept |

## THE MATRIX

| lang | Ext | Names | Shebang | Sniff | Embed | Dialect | Repair | CondComp | ErrTree | Syntax | Depth | Time | Minified | Manifest | TestFile | DirFit | notes (single most important gap) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | ✅ `.rs` | — | ⬜ | — | — | — | ⬜ | ⬜ `#[cfg]` twins both extracted (rust §07) | ✅ | ✅ | ✅ | ⬜ | 🟡 | ✅ Cargo.toml (`rust.rs:161-440`) | ✅ `tests/` `benches/` | ✅ `target/` | Module path comes from file location, not from the `mod` tree (rust §01 fixture) |
| python | ✅ `.py .pyi .ipynb` | — | 🟡 `python*` only; `#!/usr/bin/env -S uv run --script` not detected (fx:t/uvs) | — | 🟡 ipynb code cells; magics stripped by line prefix only; `.IPYNB` bypasses the extractor (fx:w) | — | ⬜ | — | ✅ | ✅ | ✅ | ⬜ | 🟡 | 🟡 `__init__.py` chain only; no PEP 420, no `pyproject`/src layout (`python.rs:139-159`) | ✅ `test_*`, `*_test`, `conftest` | ✅ `.venv` `site-packages` | Namespace packages (PEP 420) get no module segment |
| kivy | ✅ `.kv` | — | — | — | 🟡 Python in bindings handled by keyword heuristics (`kivy.rs:903-927`) | — | ✅ ERROR-suite byte scan and `harvest_unparsed_rule_calls` (`kivy.rs:341-394`, `694-778`) | — | ✅ | 🟡 `<-Foo>:`, `[Tmpl@Base]:` give `error` | ✅ | ✅ 32 KiB line cap (only language with one) | — | — | — | ✅ | Some legal KV rule forms still parse as errors |
| javascript | ✅ `.js .mjs .cjs .jsx` | — | ✅ `node`, prefix match | — | ⬜ `.vue/.svelte/.astro` and HTML `<script>` (rules.rs gaps) | — (JSX accepted by the JS grammar) | ⬜ | — | 🟡 a definition after an error is misclassified `inherentmethod` (js §01 fx f6) | ✅ | ✅ | ⬜ | ✅ | ✅ package.json, deno/jsr, tsconfig/jsconfig paths (`manifest.rs:231-290`) | 🟡 `.spec.mjs/.spec.cjs` missing (`testfile.rs:80-91`) | ✅ `node_modules` `dist` `.next` | Single-file components (Vue/Svelte/Astro) never parsed |
| typescript | ✅ `.ts .tsx .mts .cts` | — | 🟡 `ts-node` routes to JS via `node` (fixture fx:t/tsn) | — | ⬜ SFCs, as JS | 🟡 TS/TSX split is case-sensitive: `up.TS` parses with TSX → `error`, call lost (fx:v) | ⬜ | — | ✅ | ✅ | ✅ | ⬜ | ✅ (walker skip is JS-only; TS bundles are rare) | ✅ same as JS | 🟡 `.test.mts/.test.cts/.spec.mts` not test (fx:u/a.test.mts) | ✅ | `ts-node` shebang and upper-case `.TS` take the wrong path |
| go | ✅ `.go` | — | — | — | — | — | ⬜ | ⬜ build constraints unread; platform twins both bind (go §01 f12) | ✅ | ✅ | ✅ | ⬜ | — | ✅ go.mod module and `replace` (`manifest.rs:29-63`, `152-187`) | ✅ `_test.go` | ✅ `vendor/` | `//go:build` constraints ignored |
| java | ✅ `.java` | — | ⬜ JEP 330 `#!/usr/bin/java --source` (unverified) | — | — | — | ⬜ | — | ✅ | ✅ | ✅ | ⬜ | — | 🟡 `package` declaration only; `module-info.java` yields no facts (rules.rs:7110 gap) | ✅ `src/test/`, `*Test`, `*Tests`, `*IT` | ✅ `target/` `build/` `.gradle` | `module-info.java` ignored |
| kotlin | ✅ `.kt .kts` | — | ⬜ extensionless `#!/usr/bin/env kotlin` | — | — | — | 🟡 one-line top-level `object` repaired (`kotlin.rs:682`) | — | ✅ | 🟡 one-line nested class → `error`; newline call merge (kotlin §01) | ✅ | ⬜ | 🟡 | 🟡 `package` declaration | 🟡 no `*IT`, `*Spec`, `src/androidTest/` (`testfile.rs:103`) | ✅ | Test names behind Java (no `IT`); grammar newline merge |
| c | ✅ `.c .h` | — | — | ✅ `.h` content sniff and sibling rule (`detect.rs:226-327`) | — | — | 🟡 `swallowed_header` macro recovery (`c.rs:398-436`) | ⬜ all branches parsed; same-file `#if/#else` twins leave `f()` **unresolved** `ambiguous-in-file` (fx:x/c.c) | ✅ | ✅ | ✅ | ⬜ | — | — | ✅ `_test.c`, `test_*`, `_unittest.` | ✅ | No preprocessor evaluation; same-file twins make the call unresolvable |
| cpp | 🟡 no `.cu .cuh .inl .tpp .ipp .ino .c++` (cpp §01 f8/f11) | — | — | ✅ `.h` → cpp | — | — | 🟡 `clean_qualified`, prototype refusals (`cpp.rs:463-615`) | ⬜ every branch extracted; twins bind both at medium (cpp §01 f7c) | ✅ | ✅ | ✅ | ⬜ | — | — | ✅ as C | ✅ | CUDA and template-include extensions unclaimed |
| csharp | ✅ `.cs .csx` | — | ⬜ `dotnet-script` (unverified) | — | 🟡 sibling `.xaml/.axaml/.razor/.tscn` read for bindings only; Razor `@code` bodies unparsed (`markup.rs:28-44`) | — | ✅ `#if` blanking re-parse, then first branch only (`csharp.rs:683-757`) | 🟡 directives masked; both branches kept unless the parse still fails | ✅ | ✅ | ✅ | ⬜ | — | 🟡 namespaces only; `.csproj`/`.sln` never read | 🟡 `*Test.cs`/`*Tests.cs`; a `MyApp.Tests/` project dir is not test (csharp §01 f7) | ✅ `bin/` `obj/` | Razor/cshtml bodies are not C# |
| bash | 🟡 no `.zsh .ksh .bats` | ⬜ `.bashrc`, `.profile` dotfiles | 🟡 `sh` needle over-matches (pwsh, perl paths, fish, scala-cli); `zsh` needle unreachable | — | — | — (sh, dash, ksh, zsh share the Bash grammar; zsh-only syntax unverified) | ⬜ | — | ✅ | ✅ | ✅ | ⬜ | — | — | ⬜ `.bats` not detected | 🟡 `bin/` denied (`cgg-walk/src/lib.rs:57`) | Bash's `sh` needle steals other languages' scripts |
| ruby | 🟡 no `.ru .rbw .jbuilder` | ⬜ `Gemfile`, `Rakefile`, `Guardfile`, `Capfile`, `config.ru` (rules.rs:2857, 10824) | ✅ `ruby`, `irb` (subject to the `sh` hijack) | — | ⬜ ERB | — | ⬜ | — | ✅ (unverified for Ruby) | ✅ | ✅ | ⬜ | — | — | ✅ `_spec.rb`, `_test.rb` | ✅ | Rakefile, config.ru and ERB invisible |
| swift | ✅ `.swift` | — | ⬜ `#!/usr/bin/env swift` | — | — | — | 🟡 `Wrap<Int>.s()` misparse repaired (`swift.rs:297`) | ⬜ `#if os()` twins both extracted, call binds both at medium (fx:x/s.swift) | ✅ | 🟡 an additive expression is captured as the receiver (swift §01 f8) | ✅ | ⬜ | 🟡 | ⬜ Package.swift parsed as code, not read | 🟡 no `*Spec.swift` name rule (`testfile.rs:107`) | 🟡 `.build/` is hidden and walked (fx:x/.build) | SwiftPM `.build/checkouts` analyzed unless gitignored |
| lua | 🟡 no `.luau .nse .rockspec .p8` | — | 🟡 `lua`/`luajit`; any path containing `sh` goes to Bash (fx:t/luas) | — | ⬜ Lua in nginx/neovim/Redis strings (out of scope) | — | ⬜ | — | ✅ | ✅ | ✅ (lua §01 fixture) | ⬜ | 🟡 | — | 🟡 busted `*_spec.lua` outside `spec/` not test (fx:u) | ✅ | Luau not claimed |
| php | 🟡 no `.inc .module .install .php5` (php §01 p6) | — | ✅ `php` | ⬜ extensionless `<?php` (php §01 p7) | 🟡 PHP-in-HTML grammar ✅; Blade/Twig not modelled | — | ⬜ | — | ✅ | ✅ | ✅ (php §01 p6) | ⬜ | — | ⬜ `composer.json` autoload never read (grep) | ✅ `*Test.php` | ✅ `vendor/` | Drupal `.module`/`.inc` hook files invisible |
| dart | ✅ `.dart` | — | ⬜ `#!/usr/bin/env dart` | — | — | — | ⬜ | — | ✅ | ✅ Dart 3 | ✅ | ⬜ | 🟡 | ✅ pubspec.yaml `name:` (`manifest.rs:64-86`, `dart.rs:44`) | ✅ `_test.dart` | 🟡 `bin/` denied (dart §01 f2); `.dart_tool/` walked (f6) | Dart's conventional `bin/main.dart` is never analyzed |
| scala | 🟡 no `.sbt .mill` | — | 🟡 `scala`; the `scala-cli shebang` form goes to Bash (fx:t/scl) | — | ⬜ Twirl `.scala.html` (unverified) | — | ⬜ | — | ✅ | ✅ Scala 3 indentation | ✅ | ⬜ | 🟡 | 🟡 `package` declaration | 🟡 no `*Suite.scala` (`testfile.rs:118`) | ✅ | scala-cli scripts parsed as Bash |
| hcl | 🟡 no `.tf.json .tofu .nomad` (hcl §01 d) | — | — | ⬜ `.tf.json` resolves as `.json` and is content-gated, so dropped | — | — | ⬜ | — | 🟡 one unclosed resource → 0 callables (hcl §01 d/broken.tf) | ✅ | ✅ | ⬜ | — | — | — | 🟡 `.terraform/` modules are hidden and walked (inferred from `hidden(false)`; unverified) | Terraform JSON and OpenTofu files skipped |
| zig | 🟡 `.zon` not analyzed | — | — | — | — | — | ⬜ | — | 🟡 the next fn is swallowed by recovery (zig §01 p2/err.zig) | ✅ | ✅ (zig §01 fixture) | ⬜ | 🟡 | 🟡 `build.zig` literal `addModule` only (`manifest.rs:88-133`) | — (inline `test` blocks) | 🟡 `zig-out/` walked (fx:x/zig-out) | Error recovery swallows following definitions |
| objc | 🟡 `.h` never ObjC | — | — | ⬜ no `@interface` sniff for `.h`; `.m` vs MATLAB unsniffed | — | ⬜ `.mm` parsed with the ObjC grammar, so C++ is lost (objc §01 f3) | ⬜ | ⬜ (unverified) | 🟡 start line pulled up to a macro (objc §01 f6) | 🟡 `.mm` C++ constructs | ✅ (objc §01 f6) | ⬜ | — | — | ⬜ no `*Tests.m` name rule (`testfile.rs:76-119`) | ✅ | ObjC headers parsed as C, so `@interface`/`@protocol` are lost |
| r | 🟡 no `.qmd .Rprofile .Rd`; lower-case `.rmd` fails (fx:t/d.RMD) | ⬜ `DESCRIPTION`, `NAMESPACE` | 🟡 `Rscript` only; `#!/usr/bin/env R` not detected (unverified) | — | ⬜ `.Rmd` parsed as raw R → 0 callables (r §01 f6) | — | ⬜ | — | ✅ (unverified) | ✅ | ✅ | ⬜ | — | ⬜ DESCRIPTION/NAMESPACE | 🟡 generic `tests/` only | ✅ | R Markdown chunks not extracted (Python has the analogous notebook extractor) |
| groovy | 🟡 no `.gvy .gy .gsh` | ⬜ `Jenkinsfile` (rules.rs:5760) | ⬜ the grammar has a `shebang` node but nothing detects it | — | — | — | ⬜ | — | 🟡 a closure field swallows the next ctor and method (groovy §01 f1) | 🟡 untyped params, missing `;`, `trait` → `error` | ✅ | ⬜ | — | — | 🟡 `src/test/` only; no `*Spec.groovy` (fx:u) | ✅ | The grammar rejects idiomatic Groovy |
| julia | ✅ `.jl` | — | ✅ `julia` | — | ⬜ Julia-kernel notebooks are parsed as Python (`notebook.rs:11-13`) | — | ⬜ | — | ✅ (unverified) | ✅ | ✅ | ⬜ | — | ⬜ `Project.toml` (grep: no reader) | ✅ generic `test/` covers `runtests.jl` | ✅ | Julia notebooks misparsed |
| perl | 🟡 no `.cgi .psgi .plx .xs` (perl §01 f6) | — | 🟡 `perl`; `/home/shared/bin/perl` goes to Bash (f6) | — | ⬜ POD only (handled by the grammar) | — | ⬜ | — | ✅ (perl §01 f2) | 🟡 `sub :Attr`, glob assignment, qualified sub, 5.38 `class` → `error` | ✅ | ⬜ | — | — | ⬜ `t/*.t` is **not** a test file (`t` ∉ `TEST_DIRS`; no perl name rule; fx:u/t/basic.t) | 🟡 `bin/` denied; `blib/`, `local/` walked (perl §01 f1, f6) | `.t` tests and `.psgi` entry points misclassified or skipped |
| elixir | ✅ `.ex .exs` | — | ✅ `elixir` | — | ⬜ `.heex/.eex/.leex`, `~H` | — | ⬜ | — | ✅ (unverified) | ✅ | ✅ (unverified) | ⬜ | — | — (module from `defmodule`) | ✅ `_test.exs` | 🟡 `_build/` and `deps/` walked (fx:x) | HEEx templates invisible; `deps/` analyzed without a .gitignore |
| erlang | 🟡 `.escript` not detected (erlang §01 f8) | ⬜ `rebar.config`, `.app.src` | ✅ `escript` | — | — | — | ⬜ | ⬜ `-ifdef` (unverified; see category 08) | 🟡 the clause after an error is lost, its call becomes a top-level root (f8) | ✅ | ✅ | ⬜ | — | ⬜ `.app.src` `{mod,…}` unread (rules.rs:5833) | ✅ `_SUITE`, `_tests` | 🟡 `_build/` hidden and walked (inferred from fx:x for Elixir) | OTP `.app.src` entry tuple invisible |
| fortran | 🟡 no `.f77 .ftn .f18 .inc` | — | — | — | — | ⬜ fixed-form parsed with the free-form grammar | ⬜ | ⬜ raw `#ifdef` lines reach the grammar | 🟡 a function reference is silently lost (fortran §01 g/fixed.f) | 🟡 fixed-form | ✅ | ⬜ | — | — | — | ✅ | No fixed-form grammar |
| clojure | 🟡 `.bb` missing | — | ⬜ `bb` (Babashka) | — | — | — (one grammar for clj/cljs/cljc) | ⬜ | 🟡 `#?(:clj :cljs)` both branches walked at high (clojure §01 p1) | ✅ | ✅ | ✅ | ⬜ | — | 🟡 `deps.edn` parsed as code (0 callables), not as a manifest | ✅ generic | ✅ | Babashka scripts skipped |
| haskell | 🟡 no `.hsc .hs-boot` | ⬜ `.cabal`, `package.yaml` | 🟡 `runghc`/`runhaskell`; `stack`/`cabal` script shebangs not detected (unverified) | — | ⬜ `.lhs` not un-literated → `error` (haskell §01 f3) | — | ⬜ | ⬜ CPP `#else` arm **lost** → false dead (haskell §01 f5) | ✅ | ✅ | ✅ | ⬜ | — | ⬜ cabal/stack | 🟡 generic `test/` only | 🟡 `dist-newstyle/`, `.stack-work/` walked (fx:x) | CPP branch loss creates false dead-code findings |
| ocaml | 🟡 no `.mll .mly .re .rei` | ⬜ `dune` | ⬜ | — | — | ⬜ `.mli` uses the implementation grammar, `LANGUAGE_OCAML_INTERFACE` unused (ocaml §01 f1) | ⬜ | — | ✅ | ✅ | ✅ | ⬜ | 🟡 | ⬜ dune | 🟡 generic only | 🟡 `_build/`, `_opam/` walked (fx:x/_opam) | Interface grammar unused |
| powershell | ✅ `.ps1 .psm1 .psd1` | — | ⬜ declared needles `pwsh`/`powershell` contain `sh`, so they **never win**; a `.ps1` with a pwsh shebang goes to Bash (powershell §01 f9) | — | — | — | ⬜ | — | 🟡 a call after an inline `param()` is lost with no record (f8) | 🟡 ctor chaining, DSC, `./b.ps1` → `error` | ✅ | ⬜ | — | — | 🟡 Pester `*.Tests.ps1` by generic dir only | ✅ | Shebang routing to Bash, even for `.ps1` |
| solidity | ✅ `.sol` | — | — | — | — | — | ⬜ | — | ✅ (unverified) | ✅ | ✅ (unverified) | ⬜ | — | ⬜ `remappings.txt`/`foundry.toml` | 🟡 `*.t.sol` by generic `test/` only | ✅ | Foundry remappings unread |
| fsharp | ✅ `.fs .fsi .fsx` | — | ⬜ `dotnet fsi` (fsharp §01 f7) | — | — | ⬜ `.fsi` uses the implementation grammar, gives a false `val` site (f7) | ⬜ | ⬜ `#if/#else` both branches extracted (fsharp §01, §08) | ✅ (unverified) | ✅ | ✅ (unverified) | ⬜ | — | ⬜ `.fsproj` compile order | 🟡 no name rule (C# has `*Tests.cs`) | ✅ `bin/` `obj/` | Signature files misparsed |
| starlark | ✅ `.bzl .star .bazel` | ⬜ `BUILD`, `WORKSPACE`, `BUCK` (starlark §01 t/pkg/BUILD; rules.rs:6732) | — | — | — | — | ⬜ | — | ✅ (starlark §01 t4) | ✅ | ✅ | ⬜ | — | — | — | 🟡 `bazel-*` symlink dirs (unverified) | Extensionless BUILD files, where most Starlark lives |
| cmake | ✅ `.cmake` | ⬜ **`CMakeLists.txt`** (cmake §01 t1; rules.rs:6675) | — | — | — | — | ⬜ | — | 🟡 one stray `(` loses the rest of the file (cmake §01 t7) | ✅ | ✅ | ⬜ | — | — | — | ✅ `build/` | `CMakeLists.txt` is never analyzed |
| nix | ✅ `.nix` | — | — (`nix-shell` scripts go to Bash, which is accidentally right) | — | — | — | ⬜ | — | ✅ (nix §01 f4) | ✅ | ✅, plus a 64-level apply guard (`nix.rs:155-163`) | ⬜ | — | — | — | ✅ | No Nix-specific detection gap of note |
| verilog | ✅ `.v .vh .sv .svh` | — | — | — | — | 🟡 one grammar for Verilog-2005 and SV | ⬜ | ⬜ `` `ifdef `` (unverified) | ✅ (verilog §01) | 🟡 SV `class extends`, `import pkg::*`, DPI-C → `error` | ✅ | ⬜ | — | — | — | ✅ | SystemVerilog constructs break the parse |
| vhdl | ✅ `.vhd .vhdl` | — | — | — | — | — | ⬜ | — | ✅ (unverified) | ✅ VHDL-2008 | ✅ | ⬜ | — | — | — | ✅ | None in this category |
| asm | 🟡 no `.inc .nasm` | — | — | — | — | 🟡 one loose grammar for GAS, Intel, ARM and RISC-V | ⬜ | ⬜ `.S` `#define`/`#include` read as comments; `ENTRY()` lost (asm §01 f1) | ✅ (asm §01 f1) | 🟡 `.S` macros | ✅ | ⬜ | — | — | — | ✅ | Preprocessed `.S` loses its labels |
| smithy | ✅ `.smithy` | — | — | ⬜ JSON AST models (`"smithy": "2.0"`) | — | — | ⬜ | — | ✅ (smithy §01 c) | ✅ (a namespace is required by the IDL) | ✅ | ⬜ | — | — | — | ✅ | JSON AST form not detected |
| proto | ✅ `.proto` | — | — | — | — | — | ⬜ | — | ✅ (proto §01 b) | 🟡 proto2 `group`/`extensions` → `error`, `group` leaks as a reference | ✅ | ⬜ | — | — | — | ✅ | proto2 groups misparse |
| graphql | ✅ `.graphql .gql .graphqls` | — | — | ⬜ introspection JSON (unverified) | ⬜ `gql` tagged templates in JS/TS | — | ⬜ | — | 🟡 a missing `}` loses the next type (graphql §01 t3) | 🟡 legacy comma `implements` drops later interfaces (t4) | ✅ | ⬜ | — | — | — | ✅ | Embedded SDL in JS/TS never parsed |
| openapi | ✅ content-gated `.yaml/.yml/.json` | — | — | 🟡 root key must be in the first 64 lines or 2 KiB; fragment files are never analyzed (openapi §01 b/late, a/paths) | — | — | ⬜ | — | ✅ | 🟡 multi-document YAML: first document only (`structured.rs:94-101`) | ✅ (unverified) | ⬜ | — | — | — | ✅ | Split-spec fragment files lost |
| asyncapi | ✅ content-gated `.yaml/.yml/.json` | — | — | 🟡 a nested `asyncapi:` key false-positives; the first key wins against openapi (asyncapi §01 detect/nested, both) | — | — | ⬜ | — | ✅ | 🟡 multi-document: first only | ✅ | ⬜ | — | — | — | ✅ | The sniff does not check that the key is at root level |
| lean | ✅ `.lean` | — | — | — | — | — | ⬜ | — | ✅ (`mutual` defs still extracted) | 🟡 `where`, `instance (priority := …)`, `macro` → `error` (lean §01 iso/*) | ✅ | ⬜ | — | ⬜ `lakefile` | — | 🟡 `.lake/packages` hidden and walked (fx:x/.lake) | `.lake` dependency packages analyzed as workspace code |

## Parity gaps

Ranked by reach × severity. Each gap is a case where a language has the concept and a comparable language has support it lacks. Effort: S = hours, M = a day or two, L = a week or more.

1. **Bash steals other languages' shebangs.** PowerShell's own needles can never win, because `pwsh` and `powershell` both contain `sh`. scala-cli, perl, lua, fish and nix-shell scripts are all routed to Bash. Python, JS and TS are immune only because they are registered before Bash.
   - Evidence: `detect.rs:63-75` substring over the whole line; `plugins.rs:135`; fx:t/scl and fx:t/luas → `bash shebang:sh`; powershell §01 f9; perl §01 f6; bash §01 f7.
   - Fix: match the interpreter basename, or the first argument after `env [-S]`, by equality or prefix rather than substring over the line.
   - Effort: **S**.
2. **No filename-based detection anywhere.** This hits CMake (`CMakeLists.txt`), Starlark (`BUILD`, `WORKSPACE`, `BUCK`), Ruby (`Gemfile`, `Rakefile`, `config.ru`), Groovy (`Jenkinsfile`) and Bash dotfiles. It is the largest coverage hole for CMake and Bazel, since most of their code lives in those files. Comparable languages detect by extension because they have one; these have no alternative.
   - Evidence: `detect.rs:62-133` has no filename rule; cmake §01 t1; starlark §01; ruby §01; groovy §01; rule gaps at rules.rs:6675, 6732, 2857, 10824 and 5760.
   - Effort: **S** (add a `filenames()` trait method and a rule 2a). Then **M** to validate the corpus effect.
3. **Conditional-compilation handling differs per language.** C# re-parses with directives masked. C and C++ parse every branch: same-file twins bind both at medium in C++ (cpp §01 f7c), and in C the call is left **unresolved** as `ambiguous-in-file` (fx:x/c.c). Haskell keeps only the first branch, so the `#else` arm is lost and reported dead (haskell §01 f5). F#, Swift (fx:x/s.swift), Clojure `#?` and Go build tags extract every branch. Asm `.S` reads `#` lines as comments.
   - Evidence: `csharp.rs:683-757` against the other plugins, which have no equivalent.
   - Effort: **M** to extend the C# masking re-parse to Haskell, so that no branch is lost. Making same-file twins in C behave like C++ is **S** to **M**.
4. **Build-output and dependency directories are walked for the non-JS/Rust/Python ecosystems.** Hidden directories are walked, and the deny-list is tuned for node, Rust, Python and Java. These are analyzed without a `.gitignore` (fx:x): `_build/` and `deps/` (Elixir, Erlang, dune), `.lake/` (Lean), `.build/` (SwiftPM), `.stack-work/` and `dist-newstyle/` (Haskell), `_opam/`, `zig-out/`, `blib/` and `local/` (Perl), `.dart_tool/`. Meanwhile `bin/` is denied, which drops Dart's and Perl's real entry scripts and Bash tool directories.
   - Evidence: `cgg-walk/src/lib.rs:49-66`, `195`; dart §01 f2; perl §01 f1.
   - Effort: **S** for the list. **M** if `bin/` becomes language-aware, which needs a corpus re-run.
5. **Embedded-source extraction exists only for Python notebooks.** R Markdown parses as raw R and yields 0 callables (r §01 f6). Haskell `.lhs` is not un-literated (haskell §01 f3). Julia-kernel notebooks are parsed as Python. Vue, Svelte and Astro SFCs, ERB, HEEx and Razor bodies are never parsed.
   - Evidence: `notebook.rs` is Python-only (`cgg/src/lib.rs:408-423`).
   - Effort: **S** each for Rmd/qmd chunks and `.lhs` bird-track or `\begin{code}` un-literating, both offset-preserving like the notebook extractor. **M** to **L** for SFCs and templates.
6. **Interface and dialect grammars are unused.** OCaml `.mli` ignores `LANGUAGE_OCAML_INTERFACE`, and F# `.fsi` ignores `LANGUAGE_SIGNATURE` and emits a false `val` call site. ObjC `.mm` uses the ObjC grammar, so its C++ is lost. Fortran fixed-form uses the free-form grammar. TypeScript shows that the mechanism (`ts_language_for`) already works.
   - Evidence: `typescript.rs:54-61`; ocaml §01 f1; fsharp §01 f7; objc §01 f3; fortran §01.
   - Effort: **S** per language for grammar selection. **M** to make the `.mli`/`.fsi` walkers produce useful declarations.
7. **ObjC headers are always C or C++.** C and C++ get a content sniff for `.h`; ObjC gets none. `@interface`/`@protocol` in headers produce no facts, plus a spurious C call `property`.
   - Evidence: `detect.rs:270-327` has no ObjC markers; objc §01 f1 and f4.
   - Effort: **S** (an `@interface`/`@protocol`/`#import` sniff) plus **M** for ObjC-plugin handling of `.h`.
8. **Test-file naming parity.**
   - Perl `t/*.t` is not a test file at all (fx:u/t/basic.t).
   - Groovy (`*Spec.groovy`), Lua (`*_spec.lua`), PowerShell (`*.Tests.ps1`), F# and ObjC (`*Tests.*`) have no name rules, although Java, C#, Swift, Ruby and others do.
   - Kotlin lacks Java's `*IT`, Scala lacks `*Suite`, and Swift lacks `*Spec`.
   - JS/TS lack `.test.mts/.cts` (fx:u/a.test.mts) and `.spec.mjs/.cjs`.
   - C# test projects named `X.Tests/` are not matched.
   - Evidence: `testfile.rs:35-45`, `76-119`.
   - Effort: **S**.
9. **Error-recovery damage spreads past the broken construct.** One stray paren in CMake loses the rest of the file (cmake §01 t7). The next definition is swallowed in Zig, Erlang, GraphQL and Groovy. A PowerShell call is lost with **no audit record** (powershell §01 f8). A Fortran site is silently lost (fortran §01). Only C# has a re-parse, and only Kotlin, Swift, C, C++ and Kivy have targeted repairs.
   - Effort: **M** to **L** per grammar. At minimum (**S**), audit sites inside `ERROR` nodes so that nothing is silent.
10. **Extension gaps in common real-world files.** `.cu`/`.cuh`/`.inl`/`.tpp`/`.ino` for C++ (a `cuda` framework rule exists, yet `.cu` is unclaimed), `.tf.json`/`.tofu`, `.sbt`, `.luau`, `.escript`, `.bb`, `.psgi`/`.cgi`, `.inc`/`.module` for PHP, `.zon`, `.mll`/`.mly`, `.f77`.
    - Evidence: each language's §01 fixture.
    - Effort: **S** each, except where a new grammar is needed (Luau).
11. **Case-sensitive per-file paths after case-insensitive detection.** `up.TS` is detected as TypeScript but parsed with TSX, gives `error`, and its call is lost (fx:v). `NB.IPYNB` is detected as Python but skips the notebook extractor, so the JSON is parsed as Python and yields 0 callables (fx:w). `d.RMD` is skipped, because `.Rmd` is mixed-case in the list (fx:t).
    - Evidence: `typescript.rs:55-56`, `cgg/src/lib.rs:408-409`, `detect.rs:116-123`.
    - Effort: **S**.
12. **There is no parse timeout for any language.** Only Kivy has a pathological-input guard, a pre-parse line cap. Every other grammar relies on the corpus timeout (CLAUDE.md "Corpus runs are time-bounded").
    - Evidence: no `set_timeout` or progress callback in `cgg-lang/src`; `kivy.rs:85-87`.
    - Effort: **M**, because tree-sitter 0.26 progress callbacks need to be threaded through `ParserPool`.
13. **Project-manifest reading is uneven.**
    - Read: Rust (Cargo.toml), Go (go.mod), JS/TS (package.json, tsconfig, deno), Dart (pubspec.yaml) and Zig (build.zig, literal only).
    - Never read: C# `.csproj`, F# `.fsproj` (compile order matters), Swift Package.swift, PHP `composer.json` (PSR-4), Python `pyproject.toml` (src layout, PEP 420), Haskell `.cabal`, R DESCRIPTION/NAMESPACE, Julia `Project.toml`, Erlang `.app.src`, Solidity remappings.
    - Evidence: grep of `manifest.rs` and the plugins for these filenames (only `pubspec.yaml` matched).
    - Effort: **M** each. This overlaps category 04.
14. **Descriptor sniff limits.** OpenAPI and AsyncAPI need the root key in the first 64 lines or 2 KiB. AsyncAPI false-positives on a nested key. Split-spec fragments and later YAML documents are lost. Smithy's JSON AST and GraphQL introspection JSON are not sniffed at all, although the YAML/JSON content-sniff path exists.
    - Evidence: `detect.rs:160-213`; openapi and asyncapi §01; smithy §01.
    - Effort: **S** (a root-level check, a larger window) to **M** (fragment files reachable by `$ref`).
15. **Grammar coverage of legal syntax.** Groovy, Perl (attributes and 5.38 `class`), PowerShell, SystemVerilog, Lean (`where`, priority), proto2 and Kotlin (one-line nested class) all put ordinary files into `parse_status: error`.
    - Effort: **L** (grammar upgrades or vendored patches).

## Doc corrections

1. **julia.md §01 (and its scorecard "full")** says "minified or long-line files are handled by the shared gate (`lib.rs:157`, `SkipReason::LongLine` at `lib.rs:439`)". This is wrong. `LongLine` fires only when the plugin returns `Some` from `max_line_bytes` (`cgg/src/lib.rs:430-442`), and only Kivy does (`kivy.rs:85`). The minified gate only skips `locals::collect`, and Julia has no locals spec (`locals.rs:380-392`). Neither gate ever applies to Julia.
2. **vhdl.md §01** makes the same claim ("the generic long-line / minified handling (`lib.rs:157`, `:439`)"). Neither applies to VHDL, for the same reason.
3. **Several scorecards rate 01 "full" while the same gaps are rated "partial" elsewhere.** Elixir (templates invisible, `_build`/`deps` walked) and Julia (notebooks misparsed) say "full", while Ruby and R with the analogous template and embedded gaps say "partial". Under the matrix's column scheme they are partial. Rust, Go, Java and JS "full" are consistent.
4. **python.md §01** lists the shebangs as complete, but `#!/usr/bin/env -S uv run --script` (PEP 723 scripts) is not detected, because the line contains no `python` (fx:t/uvs → `unknown-extension`). The doc also says nothing about the case-sensitive notebook routing (fx:w/NB.IPYNB analyzed as plain Python, 0 callables).
5. **typescript.md §01** says `.ts/.mts/.cts` use the TypeScript grammar. That holds only for lower-case extensions: `typescript.rs:56` matches `"ts" | "mts" | "cts"` case-sensitively, so `up.TS`, although detected via `extension:.ts`, is parsed with TSX (fx:v, `parse_status: error`, call lost).
6. **CLAUDE.md** (not edited; recorded here only) says "same-file preprocessor alternatives bind once". fx:x/c.c, with `#if A void f(void){} #else void f(void){} #endif` and `g(){ f(); }` in one `.c` file, gives **no edge**. There are two unresolved records instead: `ambiguous-in-file` (file_local 2) and `candidates-in-other-files` (candidates 2). Either the claim applies only to some other shape (unverified which), or it is stale. c.md §01 does not cover this case.
7. **scala.md §01** lists the `scala` shebang as working. That holds for `#!/usr/bin/env scala`, but the common scala-cli form `#!/usr/bin/env -S scala-cli shebang` is routed to Bash (fx:t/scl → `bash shebang:sh`, `parse_status: error`).
8. **lua.md and ruby.md §01** do not mention the Bash `sh`-substring hijack, which bash.md, perl.md and powershell.md do. Lua is affected: fx:t/luas (`#!/usr/share/lua/bin/lua`) → `bash`. Ruby is registered after Bash (`plugins.rs:135-136`), so it is exposed the same way. That follows from the code and was not run for Ruby.
9. **dart.md §01** omits that `pubspec.yaml` is read during extraction (`manifest.rs:64-86`, called at `dart.rs:44`). It is the Dart counterpart of the Go and Rust manifest reads that their §01 sections do list.
10. **swift.md §01** says Swift `#if/#else/#endif` parsed `ok` (f8) but does not say what happens to the branches. fx:x/s.swift shows that both `#if os(Linux)` / `#else` copies of `p` are extracted, and `u → p` binds both at `?` (medium).
