# r

The R plugin (`RPlugin`, `crates/cgg-lang/src/plugins/r.rs`, 255 lines including 3 inline tests) parses R scripts with the crates.io grammar `tree-sitter-r` (workspace pin `"1.2"` at Cargo.toml:105, resolved to 1.2.0 in Cargo.lock:1715-1716; `tree_sitter_r::LANGUAGE`, r.rs:21-23). Nothing is vendored. It is registered 22nd in `plugins::register_all` (plugins.rs:145).

A single recursive `RWalker` (r.rs:43-199) does three things:

- It records a `binary_operator` or `equals_assignment` whose left side is an `identifier` and whose right side is a `function_definition` as a `FreeFunction` (r.rs:83-118).
- It records every `call` whose function text is not in a hard-coded builtin skip list as a `RefRecord`. The receiver is split on the last `$` or `::` (r.rs:120-198).
- It records `library(x)`, `require(x)` and `source("f")` as `ImportRecord`s of kind `"import"` or `"source"` (r.rs:128-151).

What the plugin does not do:

- It does not override `signals()`, so every optional dead-code signal is false (lib.rs:304-306): no visibility, attributes, exports, test_defs, value_refs, dyn_uses, unreachable or impls.
- It does not override `resolver_kind`, `max_line_bytes` or `ts_language_for`.
- It ignores `ExtractCtx` (`_ctx`, r.rs:27).

Resolution uses the generic pipeline. R appears by name in only two resolver places:

- the `"import"` arm of the cross-file import table (cross_file.rs:1679-1736, `"r"` at :1688);
- the `SCRIPT_DRIVEN` dead-code class list (cgg-resolve/src/deadcode/caps.rs:27-36).

R's `"source"` imports share the Bash `"source"` arm (cross_file.rs:1829-1844, include budget 4).

Fixtures cited below live under `<scratch>/parity/r/` (f1 … f15, plus `g*` import-form probes). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`. The results were dumped with `show.py` (graph) and `aud.py` (audit sidecar) in the same directory.

## 01. Detection & parsing

- **Extensions `.r`, `.R`, `.Rmd`** (r.rs:15-17). Evidence:
  - f6 `x.r` → `extension:.r`;
  - f1 `defs.R` → `extension:.R`;
  - f6 `doc.Rmd` → `extension:.Rmd`.
- **Case handling**: detect.rs:101-125 tries a case-sensitive match first and then the lowercased extension. Matching `.rmd` or `.RMD` against the list therefore finds nothing (the lowercased form `.rmd` is not listed). This is inferred from the code and was not run (unverified).
- **Shebang needle `Rscript`** (r.rs:18-20). Detection checks whether the shebang line contains the needle, iterating plugins in registration order (detect.rs:62-75). Evidence:
  - f6 `main.R` with `#!/usr/bin/env Rscript` → `shebang:Rscript`;
  - f6 extensionless `runme` → `shebang:Rscript`, parsed as R.

  `#!/usr/bin/R` or `#!/usr/bin/env R` (the `R` binary, not `Rscript`) has no needle, so it is not recognised by shebang (unverified, inferred from r.rs:18-20).
- **Not detected** (f6, `file_skipped … unknown-extension`):
  - `.Rprofile`, `.qmd` (Quarto);
  - `DESCRIPTION` and `NAMESPACE` package manifests. They are not read at all; see §04 and §12.
  - `.Rd`, `.Rhistory`, `.Rnw` (Sweave) and `.Rmarkdown` have no extension entry, so they are not detected either (inferred from r.rs:16).
- **R Markdown is parsed as raw R** (partial / effectively broken). There is no chunk extractor equivalent to `notebook::extract_python_source` for `.ipynb`.
  - f6 `doc.Rmd` (YAML front matter plus a ```` ```{r} ```` chunk containing `rmd_fn <- function() 1`) reported `parse_status: ok` but produced **0 callables**. `rmd_fn` was not extracted.
- **Content detection**: none for R. The only content sniff is the YAML/JSON descriptor sniff (detect.rs:77-97).
- **Grammar**: one grammar, no dialects, no `ts_language_for` override.
- **Re-parse/fallback**: none. A file with syntax errors is extracted as-is.
- **Depth skip**: `MAX_TREE_DEPTH` (4,000) and `SkipReason::TooDeep` are shared by every language (CLAUDE.md, cgg/src/lib.rs). There is no R-specific handling.
- **Minified / long-line handling**: `max_line_bytes` is not overridden (default `None`, lib.rs:313-315). R has no locals spec (locals.rs:380-393), so the 2,000-byte locals rule has nothing to skip.

## 02. Callables extracted

Every definition is `DefVariant::FreeFunction`, with `simple_name == qualified_name ==` the literal left-hand-side text (r.rs:97-117). Other properties of every definition:

- The span is the whole assignment expression.
- `visibility` is the empty string and `attributes` is empty (r.rs:114-115).
- `signature_hint` is `extract_signature(text)` (r.rs:113). For one-liners this holds the whole definition text; for example f1 shows `plain <- function(x) { helper(x) }`.

There is **no owner, module, package or file component** in the qualified name.

Supported shapes:

- **Left assignment** `plain <- function(x) { … }` → `plain` (f1 L1). Test `function_definitions` (r.rs:221-228).
- **Equals assignment** `eq_assign = function(y) { … }` → `eq_assign` (f1 L2). This is the `equals_assignment` node kind (r.rs:55).
- **Super-assignment** `super_assign <<- function() …` → `super_assign` (f1 L5).
- **R 4.1 lambda shorthand** `lambda_short <- \(x) x + 1` → `lambda_short` (f1 L6). The grammar gives `\(x)` the `function_definition` kind.
- **Nested definitions** `outer <- function() { inner <- function() …; inner() }` give two flat nodes, `outer` and `inner` (f1 L7-8; f7 L11-12). The walker keeps descending (r.rs:57-58). `inner` is not qualified by its parent, and calls inside `inner` are attributed to `inner` (f1 edge `inner -> helper`).
- **S3 method naming** `print.myclass <- function(x, ...)` → one free function named `print.myclass` (f1 L13; f7 `area.circle`, `area.default`). It is not split into generic and class. See §06.
- **Backtick-quoted names** are kept *with the backticks* in the qualified name:
  - `` `%+%` <- function(a, b) … `` → `` `%+%` `` (f1 L14);
  - `` `backticked name` <- function() … `` → `` `backticked name` `` (f1 L15).
- **Redefinitions** `dup <- function() 1; dup <- function() 2` give two nodes with identical QNs (f1 L24-25, f15).
- **Conditional definitions** `if (…) { condf <- function() 1 } else { condf <- function() 2 }` give two nodes, both kept (f1 L26).

Not supported (no callable produced):

- **Member assignment** `obj$method <- function() …` (f1 L12). Only an `identifier` left side counts (r.rs:94-96). This also covers `output$plot <- renderPlot(...)` and `env$f <- function`.
- **Rightward assignment** `function(x) x -> name` / `->>`. The function sits on the left-hand side, and r.rs:91 requires the RHS to be a `function_definition` (f1 L3-4: no node). The exact parse of the f1 fixture line was not inspected (partially verified).
- **`assign("dyn_fn", function() …)`** (f1 L16). This is recorded as a stdlib call to `assign`.
- **S4** `setGeneric("area", …)` / `setMethod("area", "Circle", function(shape) …)` (f1 L17-18). These are recorded only as external calls `setGeneric` / `setMethod`. The anonymous method body produces no node; its calls are top-level (`no-enclosing-callable`).
- **R6 classes** `R6::R6Class("Person", public = list(greet = function() …))` (f1 L19-22). The methods inside the `list(...)` argument produce no nodes. Their calls (`self$helper2()`, `private$secret()`) are attributed to no callable.
- **Reference classes** `setRefClass("Acc", methods = list(deposit = function(x) …))` (f7 L16-17). No node is produced for `deposit`.
- **List-literal functions** `lst <- list(a = function() 1)` (f7 L20). No node.
- **`local({ function() 1 })`** assigned to a name (f7 L21). The RHS is a call, not a function definition, so no node.
- **Anonymous functions** passed as arguments (`sapply(x, function(i) …)`, f1 L27). No node is produced, and calls inside the anonymous body have no enclosing callable unless the anonymous function sits inside a named function.
- **Scripts / top-level code as a callable**: there is no file-level node. Top-level calls are dropped from the graph (see §03, §12).
- **Abstract/interface members, constructors, properties, operators-as-methods, overload sets**: not modelled. R has no syntax for these beyond the S4/R5/R6 shapes listed above.

## 03. Call-site forms

Each `call` node produces one `RefRecord` with:

- `name` = the part after the last `$` or `::`, or else the whole function text;
- `receiver_hint` = the part before it (r.rs:184-190).

No `arity`, `arg_type`, `path_receiver`, `typed_receiver`, `value_ref`, `paren_less` or `invocant` flag is ever set (r.rs:191-197 uses `..Default::default()`).

Supported shapes:

- **Bare call** `helper(x)` → name `helper` (f1 L1, edge `plain -> helper` high intra-file). Test `calls_captured` (r.rs:245-254).
- **Namespace call** `dplyr::filter(df)` → name `filter`, receiver `dplyr` (r.rs:186-187; test `calls_captured`).
  - It is not flagged `path_receiver`, so it is treated as an ordinary receiver call. Consequences are in §04.
- **Internal namespace call** `pkg:::internal_fn()` → name `internal_fn`, receiver `pkg:` (a stray colon remains, because the split is `rfind("::")`; r.rs:186). f1 L34 → `external_calls internal_fn`.
- **`$` member call** `obj$method()` → name `method`, receiver `obj` (f1 L32). A chained `x$y$target()` → name `target`, receiver `x$y` (f9b).
- **Calls inside pipes**:
  - `plain(1) |> helper()` (f1 L35): both `plain` and `helper` are recorded.
  - `1 %>% helper()` (f1 L36): `helper` is recorded.

  The pipe's implicit first argument is not modelled. That matters only for arity, which R does not record anyway.
- **Calls nested in arguments** are recorded (walk_children after `record_call`, r.rs:61-64). For example, `suppressPackageStartupMessages(library(x))` records the outer call and treats the inner `library` as an import (`g*` probe: edge produced).
- **Call of a call** `match.fun("helper")(1)` → the function field is itself a call, so the recorded name is the literal text `match.fun("helper")` (f1 L31, `stdlib_calls match.fun("helper")`). `get("helper")(1)` gives the name `get("helper")` (f1 L38, external).
- **Builtin skip list**: `print`, `cat`, `paste`, `paste0`, `c`, `list`, `data.frame`, `matrix`, `vector`, `length`, `nrow`, `ncol`, `names`, `class`, `is.null`, `is.na`, `return`, `stop`, `warning`, `message`, plus control keywords (r.rs:152-183). These are never recorded at all: not as stdlib, and not under `--include-stdlib` (f14: `print(x)` produces no `<stdlib>` node).

Not supported:

- **User-defined infix operators** `a %+% b` (f1 L37). This is a `binary_operator`, not a `call`, so no reference is recorded and `` `%+%` `` has no caller. Built-in operator overloads (`+.myclass`) are likewise invisible.
- **Functions passed as values** `lapply(1:3, helper)` (f1 L28), `do.call(helper, …)` (f1 L30), `Reduce(f, …)`, `Map(f, …)`. The argument identifier is not recorded. `--reference-edges` adds nothing, because no `value_ref` is emitted (f14: 0 edges).
- **String-named calls** `do.call("helper", …)` (f1 L29), `match.fun("helper")`, `get("helper")`. There is no string-target capture (see §15).
- **Registrar arguments**: none. `ExtractCtx` registrar verbs are ignored (r.rs:27).
- **Arity recording**: none (`RefRecord::arity` unset).
- **Explicit generics, constructors (`new`), call operators, paren-less calls, method references**: not applicable to R syntax, or not modelled. `Foo$new()` is recorded as an ordinary `$` call named `new` (f3 L7, f7 L18 → `external_calls new`).

## 04. Imports & module resolution

Import forms recognised (r.rs:128-151). The recorded path is the first argument's value text with surrounding `"`/`'` trimmed.

- **`library(pkg)` / `library("pkg")`** → kind `"import"`, path `pkg`. Test `library_is_import` (r.rs:230-243). `library("x", character.only = TRUE)` records `x` (`g*` probe: edge produced).
- **`require(pkg)`** → kind `"import"` (r.rs:129; `g*` probe: edge produced).
- **`source("file.R")`** → kind `"source"`, path `file.R` (test `library_is_import`). The resolver handles it like a C `#include` with budget 4 (cross_file.rs:1829-1844, `collect_include_defs`):
  - f2: `source("R/util.R")` makes `util_fn` bind at **high**.
  - f2: `R/util.R`'s own `source("../sub/deep.R")` binds `deep_fn` at high from `util.R`.
  - f2: from `main.R`, a function in the twice-removed file (`fn_in_sub`) bound only at **medium** through the step-1d by-name fallback, not through the source closure. The relative path is not re-anchored at the intermediate file; this is observed behaviour, and the mechanism was not traced (unverified).
- **What `"import"` records do in resolution**: cross_file.rs:1679-1736 inserts `module_aliases[pkg] = pkg`. Their practical effect is to open the step-1d `has_imports` gate (cross_file.rs:6015-6052). With that gate open, an unresolved bare call binds by simple name to any workspace definition with ≤3 candidates whose name is not in `stdlib/r.txt`, at medium:
  - f8b: an unrelated `library(foo)` makes `target()` bind to `b.R` at medium.
  - f8: the same file with **no** import leaves `target()` unresolved (`candidates-in-other-files`, 1 candidate).

Not recognised as imports (`g*` probes, 0 edges, i.e. the gate stayed closed):

- `requireNamespace("x")` (it is recorded as a call);
- `box::use(x)` (recorded as call `use` with receiver `box`);
- `import::from(pkg, fn)`, `pacman::p_load(...)`, `sys.source("z.R")` (inferred from r.rs:129, where only three function names are special).

Other import handling:

- **Computed source paths** `source(file.path("x", "y.R"))`: the path recorded is the raw text `file.path("x", "y.R")` (r.rs:134-139). It resolves to nothing (inferred from code; not observed in the audit, which does not list imports).
- **Re-exports, aliases** (`box::use(alias = pkg)`, `@importFrom`), roxygen tags: not modelled.
- **`FileFacts::packages`**: never set. The plugin does not read `DESCRIPTION` (`Package:` field) or `NAMESPACE` (`export()`, `importFrom()`, `S3method()`). Both files are skipped as `unknown-extension` (f6).
- **Manifest reading** (`plugins/manifest.rs`): not used by R.
- **External-module refusal (heads.rs)**: R is **not** in the list (heads.rs covers Python/JS/TS/Go/Java/Kotlin/C#/C++; grep finds no `"r"` there). Consequences:
  - f3: `dplyr::filter(df)` with `library(dplyr)` and a workspace `filter` in another file binds to that workspace `filter` at **high**.
  - f2: `mypkg::pkgfn()` binds high to a `pkgfn` in an unrelated file that is not package `mypkg`.
  - f9: `nolib::target()` with no `library()` call binds at medium by duck fan-out.

  A namespace qualifier is never checked against the package the definition belongs to.
- **`not-in-imported-module` / `ModuleIndex` (modules.rs)**: no R entry. `pkg::f` is never restricted to the named package.
- **libscope.rs**: no R entry.

## 05. Type inference & receiver typing

- **Not applicable / none.** The plugin records no `local_types`, `class_fields`, `scoped_types`, return types, casts or constructions. No R code path exists in `type_hints.rs`: the only `"r"` strings there are test variable names (type_hints.rs:2883, 3033, 3693 belong to Rust/Go tests).
- **`$` receivers are untyped**: `obj$target()` duck-fans out by name across files at medium:
  - f9 `caller -> target` medium;
  - f3 `x$greet()` → `greet` medium.
- **Over-cap fan-out** is refused: f4, six `many` definitions, `obj$many()` → `fanout-cap-exceeded {candidates: 6}`.
- **Constructors are not typed**: `x <- Foo$new(); x$greet()` (f3) does not type `x`, and `R6Class`/`setRefClass` objects get no owner.
- **declared_type wrappers / nullability, closure parameter typing, implicit receivers, weak receiver types**: not applicable. R has no type annotations, and none of these are extracted.

## 06. Inheritance, dispatch & overloading

- **No class or base records.** No `ClassDecl`, bases, `trait_impl_target` or `abstract` tags are emitted.
  - S3 `class(x) <- c("child", "parent")`: not modelled.
  - S4 `setClass(contains=)`: not modelled.
  - R6 `inherit = Parent`: not modelled.
  - RC `contains=`: not modelled.
- **S3 dispatch is not modelled.** f7: `use_area <- function(c) area(c)` binds to the generic `area` (high, intra-file).
  - Nothing links `area` → `area.circle` / `area.default`. `UseMethod("area")` and `NextMethod()` are recorded as stdlib calls (f7 L7, L9).
  - `--dynamic-dispatch` adds no edge (f7: `DYN edges []`).
  - The `r-s3-dispatch` rule is a detect-only disclosure (§10).
- **S4 `setMethod` / `standardGeneric`**: recorded only as external calls (f1 L17-18). No dispatch edges.
- **`super`/`callSuper()`/`NextMethod()`, `member_of_calling_class`, `overrides_below`**: not applicable. No R branch exists in cross_file.rs; grep for `"r"` finds only the import arm.
- **`INHERITANCE_LANGS` / `dispatch::inheritance_fanout`**: R is not a member (CLAUDE.md lists Python, Java, C#, Kotlin, Scala, Groovy, TS, JS, Swift, Dart, PHP, Ruby).
- **Extension methods, mixins, traits**: not applicable.
- **Arity-based overload narrowing**: not applicable. R is not in `OVERLOAD_ARITY_LANGS`, and no arity is recorded (§03).
- **Same-file redefinition is treated as an ambiguous set, not "last wins".** f15: two `dup` definitions → `dup()` is `ambiguous-in-file` with no edge. R semantics would bind the later one.

## 07. Scoping & shadowing

- **`FileFacts::locals`: not supported.** R has no entry in `locals::spec` (locals.rs:380-393).
- **Parameters do not shadow functions** (gap / false edges):
  - f8c: `caller <- function(target) target()` with `source("b.R")` binds `target()` to `b.R`'s `target` at **high**.
  - f7: `use_param <- function(cb, helper) { helper(); cb() }`: no import, so the calls are left unresolved (`candidates-in-other-files`). With any import they would bind by name.
- **Locals do not shadow**: `local_var <- other; local_var()` records `local_var` as an external call (f7 L4-5). This is correct only by accident.
- **Lexical narrowing of nested definitions: not applied.** R is not in `lexically_scoped` (intra_file.rs:119-135).
  - f7: `calls_inner <- function() inner()` binds at **high** to the `inner` nested inside `outer1`, which is not visible from `calls_inner` in R (false edge).
  - Two nested `inner`s in different files each bind within their own file (f7 `outer2 -> inner`) only because they are in different files.
- **Nested redefinition**: f15 `use2 <- function() { dup <- function() 3; dup() }` is `ambiguous-in-file`, so the local definition does not win.
- **Positional shadowing**: none.
- **Conditional definitions**: both branches are kept as separate nodes (f1 `condf` ×2). There is no `DefRecord::conditional` marking. A call to `condf()` would be `ambiguous-in-file`, inferred from f15 (unverified for this exact shape).

## 08. Macros, preprocessor & codegen

- **Not applicable: R has no macro or preprocessor layer.** No `macro:<name>` tagging and no `c_macro`/`rust_macro` analogue.
- **Non-standard evaluation / quoting** (`quote()`, `bquote()`, `substitute()`, `rlang::expr`, `!!`, `{{ }}`): not modelled. Calls inside `quote(...)` are recorded as ordinary calls, because the walker descends into every argument (r.rs:61-64). This is inferred from code and was not run (unverified).
- **Generated code** (roxygen2 `NAMESPACE`/`.Rd` output, `Rcpp::compileAttributes()`'s `RcppExports.R`): `RcppExports.R` is plain R and would be parsed like any other file, but no generated-code tag exists.
- **`FileFacts::name_mentions`**: never written by the R plugin.

## 09. FFI & cross-language links

- **None.** `ffi::link_ffi` has no R arm (grep for `Rcpp`, `.Call` and `reticulate` in cgg-resolve/src and the plugins: no hits). Unmodelled forms:
  - `.Call("C_fn", …)`, `.C(...)`, `.External(...)`;
  - Rcpp `// [[Rcpp::export]]` C++ functions and the generated `RcppExports.R`;
  - `reticulate::import("mod")`, `py_run_string`.

  All are ordinary R calls with no cross-language edge.
- **Descriptor links (protobuf rpc → host)**: none for R.
- **Markup code-behind** (`markup.rs`): not applicable.

## 10. Frameworks & entry points

Five rules in the `SPECS` table name `language: "r"` (rules.rs:6209-6320). Detection is **repository-wide**: a `library()`/`require()` import of a `detect` package in *any* R file activates the rule for every R file.

- f13: `library(rlang)` in `a.R` activates `.onLoad` in `zzz.R`.
- f10: with no import anywhere, nothing is minted.

Synthetic detection evidence comes from test `crates/cgg/tests/detect_prefixes.rs:71`, which writes `library({prefix})` into a `.R` file.

| rule id | kind | trigger | enumerates? |
| --- | --- | --- | --- |
| `r-package-hooks` (rules.rs:6210) | lifecycle | detect: any of `methods, utils, stats, tools, grid, graphics, grDevices, testthat, rlang, vctrs, glue, withr, R6, data.table, tidyverse`; methods: `.onLoad, .onAttach, .onUnload, .onDetach, .First.lib, .Last.lib` | **Yes**: mints `<framework-entry>::lifecycle::r-package-hooks::<name>` with a `framework_entry` edge (low) to each matching function (f5: `.onLoad`, `.onAttach`; f13: `.onLoad`, `.onUnload`, `.Last.lib`; f12) |
| `testthat` (rules.rs:6250) | test | detect `testthat` | No: detect-only. The gap text says `test_that("desc", { … })` has no named callable. Reported under "seen, no rules" (f5) |
| `shiny` (rules.rs:6267) | network | detect `shiny, shinydashboard, shinyWidgets, golem, bslib` | No: detect-only. The gap covers `observeEvent`, `reactive`, `renderPlot`, `moduleServer`. `server <- function(input, output, session)` is reported dead (f5) |
| `plumber` (rules.rs:6284) | network | detect `plumber` | No: detect-only. `#* @get /path` comment annotations are not read. `echo` is reported dead (f5) |
| `r-s3-dispatch` (rules.rs:6301) | lifecycle | same detect list as `r-package-hooks` | No: detect-only disclosure that S3/S4/S7 methods are reached by dispatch (f5 coverage: "found in 2 file(s), entries NOT enumerated") |

- **No multi-language rule** includes R (grep for `languages:` in rules.rs finds no list form containing `"r"`).
- **`member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, visibility entries**: none for R, because R definitions carry no attributes or visibility.
- **Registrar verbs**: none. Shiny/plumber registrars are disclosed as gaps.
- **Benchmark coverage** (scripts/benchmark.sh `APPS`):
  - `r-ggplot2` (:233): `r-package-hooks`, `~r-s3-dispatch`, `~testthat`;
  - `app-arcos-plumber` (:290): `~r-package-hooks`, `~plumber`;
  - `app-tweetconf-shiny` (:302): `~shiny`;
  - `r-package-hooks` is also seen in `app-dataverse-jsf` (:273), `app-basedosdados-prefect` (:291) and `scala-spark` (:397).

  A `~` prefix marks a rule as expected detect-only.

## 11. Test harness entries

- **testthat has no enumeration.** The rule exists (`kind: test`, rules.rs:6250-6264) but mints no harness node, because `test_that(...)` blocks are anonymous brace expressions.
  - Calls inside `test_that` are top-level: f5 `setup_cache()` inside `test_that` in `tests/testthat/test-a.R` stays unresolved (`no-candidate-in-file`) and creates no edge.
  - In the same-file case, the `toplevel:invocation` root (§12) could still credit the callee.
- **Test-file gating**: no R-specific file-name rule in `classify_test_file`. There is no `test-*.R` pattern (testfile.rs:77-121, `_ => false`). Only the generic directory rule applies (`tests`, `test`, … ; testfile.rs:36-46, 125-132).
  - f5: `helper_in_test` in `tests/testthat/test-a.R` gets `test_role: support` through the directory rule.
  - A `test-foo.R` outside a test directory would not be classified (inferred from code).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`**: not applicable (Python/PHP/Ruby only).
- **Annotation / naming-pattern test rules, benchmark and fuzz harnesses** (`bench::mark`, `microbenchmark`): none for R.
- **`--no-test-entries`**: has nothing to convert for R.

## 12. Dead-code analysis

- **Language class `script-driven`** (caps.rs:27-36, 165-166):
  - `max_confidence: medium` (f5 capabilities JSON);
  - the "depends on roots" categories are withheld (deadcode/mod.rs:811-812);
  - every finding carries evidence `language-is-script-driven`.

  Observed: all findings are **low**, and `reachable-only-from-dead-code` is "withheld (script-driven language)" (f5, f6).
- **Signal coverage** (f5): `visibility none, attributes none, value_references none, dispatch none, exports none, test_tagging none`. Each finding lists `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-value-references` and `language-lacks-visibility` (f5 evidence).
- **Root kinds that fire for R**:
  - `framework:entry-node`: `r-package-hooks` entries (f5 roots: `.onLoad`, `.onAttach`).
  - `toplevel:invocation` (roots.rs:552-612). A callable whose simple name matches a top-level call site with reason `no-enclosing-callable` becomes a root. f6: `local_fn()` at top level of `main.R`, defined in the same file → `ROOT local_fn toplevel:invocation`.
- **Gap: a top-level call to a function in another file is not a root.** f6: `main.R` does `source("lib.R")` and calls `lib_fn()` at top level. The site's reason is `no-candidate-in-file` (not `no-enclosing-callable`), so `lib_fn` is reported **dead** (`FIND lib_fn low never-referenced`). This is the normal shape of an R script driving a sourced library.
- **No exported/public roots**: `NAMESPACE` `export()` / roxygen `@export` are not read, so every exported package function looks unreferenced unless something in the tree calls it.
- **S3 methods** (`print.myclass`) are reported dead (f5). `r-s3-dispatch` only discloses this gap and does not root them.
- **Plumber handlers and the Shiny `server`** are reported dead (f5).
- **Confidence caps `implements-external-contract`, `macro-generated`, `reflection`**: none apply. R has no dynuse screen (§15) and no contract/macro tags.
- **Unresolved-doubt handling**: generic. A finding records `no-unresolved-site-with-name` when no unresolved site shares its name (f5 evidence).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/r.txt`: 321 names**, one per line with no comments (`grep -c .` = 321). It is loaded at stdlib.rs:198 and registered at stdlib.rs:239. Examples: `abline, abs, apply, as.character, sapply, lapply, assign, get, parse, local, UseMethod, NextMethod, Recall, invisible, match.fun`, as observed in the f1/f7 `stdlib_calls` buckets.
- **What it does**:
  - Unresolved calls with these names land in the `stdlib_calls` audit bucket instead of `external_calls` (f1, f7).
  - Step 1d skips stdlib names (cross_file.rs:6022-6023).
  - A same-file user definition still wins: f9 `apply <- function(x) x` then `apply(1)` → `caller -> apply` high.
- **Plugin-level skip list** (r.rs:152-183): 26 names that are never recorded at all (§03). This is separate from `r.txt`.
- **Namespace-qualified stdlib calls are classified as external, not stdlib**: f2 `stats::median(1)` → `external_calls median`.
- **`--include-stdlib` / `--include-external`** mint `<stdlib>::name` / `<external>::recv::name` leaf nodes (f14: `<stdlib>::sapply`, `<external>::dplyr::filter`, `<external>::x::m`). Skip-listed names (`print`) never appear.
- **`rust_types`-like tables, `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, implicit packages**: none for R.

## 14. Confidence & certainty

- **high** appears for:
  - same-file unique-name binds (f1 `plain -> helper`, f7 `use_area -> area`);
  - a single candidate reached through the `source()` include closure (f2 `run -> util_fn`, `util_fn -> deep_fn`; f8c);
  - a `pkg::name` receiver with one workspace candidate (f2 `run -> pkgfn`, f3 `go -> filter`).

  The last case is **structurally wrong**, because the package qualifier is never verified (§04).
- **medium** appears for:
  - step-1d by-name fallback when the file has any import (f2 `run -> unsourced_fn`, f8b);
  - untyped `$` duck fan-out (f3, f9);
  - multi-candidate fan-out (f3 `mutate` ×2).
- **low**: `framework_entry` edges (f5).
- **No edge**: same-file duplicates (`ambiguous-in-file`, f15), over-cap fan-out (`fanout-cap-exceeded`, f4), and files with no import (`candidates-in-other-files`, f8).
- **False highs** occur from missing shadowing and scoping (§07):
  - f8c: a parameter call binds to a sourced function;
  - f7: a call binds to a nested `inner` of another function.
- **Mermaid `?` marker**: works generically. f3 mermaid renders `go -->|?| mutate` and `go -->|?| greet` for medium-only arrows, and `go -->|3x| filter` for an arrow with a high site.
- `--guess-unique-names` is Rust-only and has no effect for R (CLAUDE.md).

## 15. Dynamic use & reflection

- **No dynuse coverage.** `dynuse` covers Python, JS, Java, C#, Ruby and PHP only (CLAUDE.md). The plugin emits no `dyn_uses` (default `signals()`).
- Each of the following is recorded only as a plain stdlib or external call, with no edge, no doubt cap and no `reflection` screen:
  - `get("helper")(1)` → name `get("helper")` external (f1 L38);
  - `match.fun("helper")(1)` (f1 L31);
  - `do.call("helper", …)` (f1 L29);
  - `eval(parse(text = "helper(1)"))` (f1 L39: `eval` external, `parse` stdlib);
  - `exists("f")`, `getFromNamespace`, `environment` lookups (inferred, not run).
- **Module loads** (`requireNamespace`, `loadNamespace`, `library(x, character.only = TRUE)` with a variable): `character.only` with a literal records the literal as an import (§04). There is no `module-load` screen.

## 16. Known gaps & limitations

Gaps the code itself discloses:

- `gap` strings in rules.rs for `testthat` (:6262), `shiny` (:6279), `plumber` (:6296) and `r-s3-dispatch` (:6313). Each states that the R plugin records no argument/context metadata or comment annotations.
- `SCRIPT_DRIVEN` (caps.rs:27-36) caps R dead-code findings below high.

Gaps found while cataloguing:

- **Namespace qualifier not enforced**: `pkg::f()` binds by bare name to any workspace `f`, at **high** when unique (f2, f3). There is no heads.rs/modules.rs entry for R and no `DESCRIPTION` reading.
- **Cross-file resolution is gated on having any `library`/`require`/`source`** (f8 vs f8b). R packages, whose `R/*.R` files share one namespace with no imports between them, therefore get **no cross-file edges** for bare calls from import-free files.
- **No parameter/local shadowing and no lexical scoping** (f7, f8c): false high edges.
- **Redefinitions are ambiguous rather than last-wins** (f15).
- **Methods defined inside calls are invisible**: R6 `R6Class(public = list(...))`, `setRefClass(methods = list(...))`, S4 `setMethod(...)`, `obj$f <- function`, `list(f = function)` (f1, f7).
- **No S3/S4/R6 dispatch model**: `print.myclass`-style methods are always dead and never linked to their generic (f5, f7).
- **User infix operators** (`%op%`) are never recorded as calls (f1 L37).
- **No value references**: `lapply(x, f)`, `do.call(f)`, `Reduce(f, …)`, and Shiny/plumber handler registration (f1, f14).
- **`.Rmd` is parsed as raw R and yields nothing**. `.qmd`, `.Rprofile` and `NAMESPACE` are not detected (f6).
- **Top-level calls to functions in other files are not roots** (f6 `lib_fn` reported dead).
- Other defects:
  - Backtick names keep their backticks in the QN (f1).
  - `pkg:::f` leaves the receiver `pkg:` (f1).
  - `match.fun("x")(…)` records the literal call text as the name (f1).
  - `source()` with a computed path records the expression text (r.rs:134-139).
- **Tests**: only 3 inline unit tests (r.rs:221-254), plus the synthetic detection test `detect_prefixes.rs:71`. There are no R fixtures under `crates/*/tests` and no ignored tests.
- **README** language-table row (README.md:500): "R | library(), source() | — | `<-` and `=` assignment". The benchmark row is `ggplot2 | r | 946 | 419 | 3% | 66ms` (README.md:1326), from `r-ggplot2` (benchmark.sh:74).
- **CHANGELOG**: no R-plugin-specific entries were found by grepping for `R plugin`, `.R` and R framework names. The only hit is a rendering check naming `r-ggplot2` and `app-tweetconf-shiny` (CHANGELOG.md:1713).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | partial | `.r/.R/.Rmd` + `Rscript` shebang; `.Rmd` parsed as raw R (0 callables); `.qmd`/`.Rprofile`/NAMESPACE not detected |
| 02. Callables extracted | partial | `<-`, `=`, `<<-`, `\(x)` to an identifier only; no R6/RC/S4/`obj$f<-`/list-member methods; backticks kept in QN |
| 03. Call-site forms | partial | bare, `pkg::f`, `$` calls; no infix `%op%`, no value refs, no arity, 26-name hard skip list |
| 04. Imports & module resolution | minimal | library/require/source; source = include closure (budget 4); `pkg::` not enforced, no external refusal, no DESCRIPTION/NAMESPACE |
| 05. Type inference & receiver typing | none | no typing; `$` receivers duck-fan out by name |
| 06. Inheritance, dispatch & overloading | none | no S3/S4/R6 class or dispatch model; not in INHERITANCE_LANGS/OVERLOAD_ARITY_LANGS |
| 07. Scoping & shadowing | none | no locals spec, not lexically scoped; parameters and nested defs produce false high edges |
| 08. Macros, preprocessor & codegen | n/a | no macro layer; NSE/quote not modelled |
| 09. FFI & cross-language links | none | no `.Call`/Rcpp/reticulate links |
| 10. Frameworks & entry points | minimal | 5 rules; only `r-package-hooks` enumerates (`.onLoad` etc.); shiny/plumber/s3 detect-only |
| 11. Test harness entries | minimal | testthat detect-only; test files only via generic `tests/` directory rule |
| 12. Dead-code analysis | minimal | script-driven (max medium, observed low); top-level cross-file calls not roots; no export roots |
| 13. Builtins & stdlib tables | partial | `stdlib/r.txt` 321 names + plugin skip list; `pkg::` stdlib calls classified external |
| 14. Confidence & certainty | partial | high from same-file/source closure, but false highs from unverified `pkg::` and missing shadowing |
| 15. Dynamic use & reflection | none | no dynuse; get/match.fun/do.call/eval are plain calls |
| 16. Known gaps & limitations | n/a | documented above; 3 unit tests, no R fixtures |
