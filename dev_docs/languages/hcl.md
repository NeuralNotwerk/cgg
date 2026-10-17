# hcl

The HCL plugin (`HclPlugin`, `crates/cgg-lang/src/plugins/hcl.rs`, 188 lines) covers Terraform, Terragrunt, Packer, Nomad and other HCL dialects. The grammar is the crates.io `tree-sitter-hcl` crate (`Cargo.toml:102`, `tree-sitter-hcl = "1.1"`; `crates/cgg-lang/Cargo.toml:40`). Nothing is vendored. It is registered at `crates/cgg-lang/src/plugins.rs:142`. The module doc says it does "dependency graph extraction": HCL has no user-defined functions, so a **labelled block** (`resource "t" "n"`, `module "m"`, `variable "v"`, …) becomes a definition and a **`function_call`** node becomes a reference (`hcl.rs:1-3`). The `LanguagePlugin` trait has no `resolver_kind` method (`grep resolver_kind crates` finds nothing, so CLAUDE.md is stale on this point). The plugin does not override `signals()` (default all-false, `crates/cgg-lang/src/lib.rs:304`), `max_line_bytes()` (None) or `ts_language_for()`. Every HCL file goes through the generic intra-file and cross-file resolvers, and HCL has no language-specific code anywhere in `cgg-resolve` or `cgg-core` except two entries: the `hcl.txt` stdlib table (`crates/cgg-core/src/stdlib.rs:203,244`) and the `SCRIPT_DRIVEN` dead-code class (`crates/cgg-resolve/src/deadcode/caps.rs:31`).

## 01. Detection & parsing

- **Extensions** are `.tf`, `.hcl` and `.tfvars` (`hcl.rs:17-19`).
  - Fixture `parity/hcl/a`: `main.tf` was detected via `extension:.tf`, `terragrunt.hcl` and `job.nomad.hcl` via `extension:.hcl`, and `x.tfvars` via `extension:.tfvars`.
  - Compound extensions ending in `.hcl` are detected: `p.pkr.hcl` (Packer) was analyzed as hcl (fixture `parity/hcl/d`). `*.tftest.hcl` also ends in `.hcl`, so it is detected the same way by extension (inferred from the rule, not run).
- **Shebangs**: none (`hcl.rs:20-22`).
- **Content detection**: none. HCL has no entry in `detect.rs`; the `.yaml`/`.json` content rule there serves OpenAPI/AsyncAPI only.
- **Not detected**: fixture `parity/hcl/d` skipped all three of these as `unknown-extension`:
  - `m.tf.json` (Terraform JSON syntax)
  - `m.tofu` (OpenTofu)
  - `j.nomad` (legacy Nomad extension)
- **Grammar**: a single grammar, `tree_sitter_hcl::LANGUAGE` (`hcl.rs:23-25`). There is no per-file dialect choice.
- **Re-parse or fallback**: none. A file with a syntax error got `parse_status: "error"`, and no definitions were recovered from it. In fixture `d/broken.tf`, an unclosed `resource "a" "b" { x = upper(` gave 0 callables.
- **Skip conditions** are the generic ones:
  - Depth cap: `cgg_lang::exceeds_depth` against `MAX_TREE_DEPTH = 4000` gives `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155,451-454`; `crates/cgg-lang/src/parser.rs:112`).
  - No `max_line_bytes` override, so there is no `LongLine` skip.
  - The minified-line rule (`MINIFIED_LINE_BYTES = 2000`, `lib.rs:159`) only gates locals collection, and HCL has no locals (see 07).

## 02. Callables extracted

- **Labelled block → `DefVariant::FreeFunction`** (`hcl.rs:84-121`). The walker collects the block's leading `identifier` and `string_lit` children (quotes trimmed) up to the body. A definition is produced **only when there are ≥2 labels**, block type included (`hcl.rs:101`). Each definition gets:
  - `qualified_name`: the labels joined with `::`
  - `simple_name`: the last label
  - `signature_hint`: the block header, via `extract_signature`
  - `visibility`: empty
  - `attributes`: none
  - `kind` in JSON: `function`

  Fixture `parity/hcl/a` (all confirmed):

  ```hcl
  resource "aws_s3_bucket" "b" { ... }   # resource::aws_s3_bucket::b  (simple: b)
  data "aws_ami" "ubuntu" {}             # data::aws_ami::ubuntu
  module "vpc" { source = "./modules/vpc" }  # module::vpc
  variable "region" {}                   # variable::region
  output "bucket_id" {}                  # output::bucket_id
  provider "aws" {}                      # provider::aws
  include "root" {}                      # include::root   (terragrunt)
  job "web" { group "g" { task "t" {} } }  # job::web, group::g, task::t (nomad)
  source "amazon-ebs" "u" {}             # source::amazon-ebs::u (packer, fixture d)
  check "health" {}                      # check::health  (fixture c)
  ```

- **Nested labelled blocks** become separate, flat callables. The parent's labels are not part of the qualified name: `dynamic "rule"` inside a resource became `dynamic::rule`, and Nomad `group "g"` became `group::g`, not `job::web::group::g` (fixtures a and c). The walker does not track scope (`hcl.rs:57-60`).
- **Unlabelled blocks are not callables** (fewer than 2 labels):
  - `terraform {}`, `locals {}`, `lifecycle {}`, `content {}`, `required_providers {}`, `assert {}`, `moved {}`, `import {}`, `removed {}`, `build {}` (Packer)
  - Fixtures a, c and d: none appear in the callables list.
  - Call sites inside these blocks belong to the nearest labelled ancestor. If there is none they have `src: null` (see 03).
- **Top-level attributes** are not callables. This covers `inputs = {...}` in terragrunt and every assignment in a `.tfvars` file (fixture a). As a result, `.tfvars` files never produce callables.
- **Same-named blocks** with identical labels in different files are separate nodes. `variable::name` in a child module and in the root module are distinct (fixture a); neither the module directory nor the file appears in the QN.
- **Not applicable**: methods, constructors, closures and lambdas, properties, operators, overloads, abstract members. HCL has none of them.
- **Not modelled**:
  - Terraform `locals` entries (`local.x` named values) are not callables.
  - Neither are `for` expressions.
  - Neither are provider-defined functions (`provider::aws::arn_parse`); only their call sites are seen.

## 03. Call-site forms

- **Bare function call** `lower("x")`: the `function_call` node's first child `identifier` is recorded as a `RefRecord` with the name only, an empty `receiver_hint`, and the site line and byte (`hcl.rs:123-139`).
- **Calls in every expression position** are captured, because the walker recurses through all children (`hcl.rs:62-66,72-82`). Fixture c confirmed edges from each of:
  - Template interpolation: `"${upper("a")}-${myfn(2)}"`
  - Heredoc interpolation: `<<EOT ${join(",", ["a"])} EOT`
  - `for` expression body and condition: `[for s in var.l : title(s) if length(s) > 0]`
  - Conditional: `var.c ? abs(-1) : max(1, 2)`
  - Nested: `try(jsondecode(file("x")), {})`, giving try, jsondecode and file
  - Expansion: `myfn(var.l...)`
  - Nested non-labelled block: `can(regex(...))` inside `check { assert {} }` attributed to `check::health`
- **Provider-defined functions** `provider::aws::arn_parse("x")`: recorded under the full name `provider::aws::arn_parse` as one identifier, with no receiver split. With `--include-external` it became `<external>::provider::aws::arn_parse` (fixture a).
- **Calls with no enclosing labelled block** have `src: null` in the audit. This covers `locals { name = lower(...) }`, top-level `inputs = { x = get_env(...) }` and `.tfvars` values (fixture a audit: `get_env` and `lower` with `"src": null`). They are counted in the stdlib/external buckets but cannot become edges.
- **Not recorded**:
  - **Traversals and references**: `var.region`, `local.name`, `aws_s3_bucket.b.id`, `data.aws_ami.ubuntu.id`, `module.vpc.x`, `each.value`, `count.index`. These are how Terraform expresses dependencies; the plugin only matches `block` and `function_call` (`hcl.rs:56-67`). Fixture a: `output "bucket_id" { value = aws_s3_bucket.b.id }` gives no edge.
  - `depends_on = [aws_s3_bucket.b]` gives no edge (fixture a).
  - `module "vpc" { source = "./modules/vpc" }`: no link to the child module's files (fixture a: no edge).
  - `moved`, `import` and `removed` `from`/`to` traversals give no edge (fixture c).
  - Arity (`RefRecord::arity`), `value_ref`, `paren_less`, `path_receiver`, `typed_receiver`, registrar arguments, explicit generics: none are set (`hcl.rs:130-136` uses `..Default::default()`).

## 04. Imports & module resolution

- **None.** The plugin records no `imports`, `packages`, re-exports or aliases (`hcl.rs` never touches `FileFacts::imports`/`packages`).
- **Effect on cross-file resolution**: cross-file step 1d (global by-simple fallback) runs only when the file `has_imports` (`crates/cgg-resolve/src/cross_file.rs:6015-6022`). HCL files never have imports, so a name never binds across files.
  - Fixture b: `myfn(1)` in `b.tf`, with `resource "null_resource" "myfn"` in `a.tf`, is unresolved with reason `candidates-in-other-files {candidates: 1}` (`cross_file.rs:2869-2886`).
  - This matches the CHANGELOG and skill note that HCL "genuinely yields" no cross-file edges (`CHANGELOG.md:5471`; `skills/cgg/SKILL.md:501`). The README language table shows `—` for both import and type columns (`README.md:505`).
- **Terraform `module` `source`**, Terragrunt `include`/`find_in_parent_folders`/`dependency`, `terraform_remote_state` and `templatefile` paths are not resolved to files (fixture a).
- **Not applicable**: `heads.rs` external-module refusal (`WorkspaceVocab`/`FileHeads` cover Python/JS/TS/Go/Java/Kotlin/C#/C++ per CLAUDE.md), `modules.rs` `not-in-imported-module`, manifest reading, `libscope.rs`. A grep for `hcl` in `crates/cgg-resolve/src` finds only `deadcode/caps.rs`.
- **Unrelated mention**: `cross_file.rs:1478` mentions terraform-provider-aws only as a performance anecdote about the include index for Go-heavy repos.

## 05. Type inference & receiver typing

- **Not applicable.** HCL calls have no receiver, and the plugin records no parameter types, local types, field types, return types or `scoped_types`. `type_hints` has no HCL entry (grep: no `"hcl"` in `crates/cgg-resolve/src`).
- Terraform `variable { type = string }` declarations are not read as types (fixture a: `variable::region` has no attributes or type info).

## 06. Inheritance, dispatch & overloading

- **Not applicable / none.**
  - No base records, super calls or `member_of_calling_class`.
  - HCL is not in `INHERITANCE_LANGS` or `OVERLOAD_ARITY_LANGS` (grep: no `hcl` in `crates/cgg-resolve/src`).
  - `--dynamic-dispatch` adds nothing, because no `trait_impl_target` is set.
- Duplicate qualified names in one file, such as two `variable "x"` blocks from an invalid configuration, would form a generic same-QN set (unverified; no HCL-specific handling).

## 07. Scoping & shadowing

- **No `FileFacts::locals`.** `cgg_lang::locals::collect` has no HCL branch. Its language list is javascript, typescript, python, lua, scala, kotlin, swift, rust, dart, ocaml and zig (`crates/cgg-lang/src/locals.rs:340-392`; `_ => return None` at :392).
- **Consequence, a false `high` edge**: within one file, a function call binds to a block whose *last label* equals the function name. Fixture c had `resource "null_resource" "myfn" {}` and, in the same file, `x = myfn(1)`. The result was three edges `resource::null_resource::caller -> resource::null_resource::myfn`, `high`, resolver `intra-file`. A Terraform function call can never target a resource block, so these edges are always wrong. The same would happen for a block named like a built-in, for example `resource "x" "lower"` with `lower()` in the same file.
  - In fixture b, `lower` in another file left `lower("A")` **unresolved** (`candidates-in-other-files`) instead of filing it as stdlib.
- No lexical narrowing, positional shadowing or conditional definitions. `for` expression iterators (`for s in ...`) and `each`/`count` are not recorded as bindings.

## 08. Macros, preprocessor & codegen

- **None.**
  - No macro expansion, `macro:<name>` tagging or `name_mentions`.
  - Terraform `dynamic` blocks, the closest analogue to generated blocks, are extracted as an ordinary callable `dynamic::<label>` (fixture a).
  - Template files (`templatefile("x.tpl")`, `.tftpl`) are not parsed (no extension registered).

## 09. FFI & cross-language links

- **None.**
  - No HCL entry in `ffi::link_ffi` or `descriptor::link_descriptors`; HCL is not in the `DESCRIPTOR` list (`deadcode/caps.rs:40`).
  - No markup code-behind.
  - A Terraform resource is never linked to the provider's Go implementation, even in a repo such as terraform-provider-aws that contains both (benchmark entry `scripts/benchmark.sh:369`, where `hcl-terraform-aws` is detected via Go frameworks only).

## 10. Frameworks & entry points

- **No framework rule applies to hcl.** `grep -n '"hcl"\|terraform' crates/cgg-core/src/frameworks/rules.rs` returns nothing. There is no multi-language or wildcard rule, and no member_sources, signature_methods, container_attributes or markup_kinds.
- Framework coverage on fixture a reported `"no_markers": [{"language": "hcl", "files": 5}]` and `nodes_minted: 0`.
- `crates/cgg/tests/detect_prefixes.rs:94,179` explicitly skips hcl, saying descriptor/config languages "have no import statement to synthesize".
- Terraform root-module conventions (`output`, `variable`, `provider`, `terraform` blocks) and Terragrunt/Packer/Nomad roots are not modelled as entries.
- No registrar verbs.

## 11. Test harness entries

- **None.**
  - No test rule for `terraform test` (`*.tftest.hcl` with `run "name" {}` blocks), Terratest (Go, not HCL) or Packer/Nomad validation.
  - A `run "x" {}` block in a `.tftest.hcl` would be a plain `run::x` callable (unverified by fixture; it follows from the ≥2-label rule).
  - No `TestRole`, because `signals().test_defs` is false (default).
  - Not in `TEST_CLASS_MEMBERS_ARE_ROOTS`.

## 12. Dead-code analysis

- **Class `ScriptDriven`**: `hcl` is in `SCRIPT_DRIVEN` (`crates/cgg-resolve/src/deadcode/caps.rs:27-36`), with the comment "primary invocation path is top-level script code that cgg does not model as a callable". The class is assigned at `caps.rs:165-166`.
- **Root-dependent findings are suppressed** with `SuppressionReason::ScriptDriven` (`crates/cgg-resolve/src/deadcode/mod.rs:811-812`), and receive `Evidence::LanguageIsScriptDriven` (`mod.rs:927`), which caps confidence at **Low** (`crates/cgg-core/src/deadcode.rs:314-315`).
- **Fixture b `--dead-code`** shows:
  - `roots 0 discovered · root-reachability disabled:no-roots-found`
  - signal row `hcl  3 files  4 callables  script-driven  vis no  attrs no  refs no  disp no  reach 0%`
  - `findings 0 shown (high) · 4 withheld (low)`
- No visibility, attributes, exports, value refs, dyn uses, unreachable-code or impls signals (`signals()` default, `crates/cgg-lang/src/lib.rs:304-306`).
- There are no public or exported roots. In practice every HCL block is "unreferenced" because references (`var.x`, `aws_x.y.id`) are never recorded (see 03), so withholding is the only honest outcome.

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/hcl.txt`**: 125 names, one per line, all lowercase identifiers (`abs` … `zipmap`), loaded at `stdlib.rs:203,244`.
  - Calls whose names are in it go to the stdlib bucket (`name_screen_applied: "stdlib"`). With `--include-stdlib` they become `<stdlib>::<name>` exit nodes, joined by `low` edges with `via: stdlib` and resolver `exit-node` (fixture c: upper, title, length, abs, max, join, templatefile, try, jsondecode, file, can, regex).
  - The CHANGELOG lists hcl among the 21 stdlib lists "tuned against real-world `external`-bucket noise" (`CHANGELOG.md:5971-5973`).
- **Missing modern Terraform functions** (grep -cx = 0): `strcontains` (1.5), `issensitive` (1.8), `ephemeralasnull` (1.10), `templatestring` (1.9), `cidrcontains`. Calls to these land in the **external** bucket instead of stdlib.
- **Terragrunt built-ins** (`find_in_parent_folders`, `get_env`, `read_terragrunt_config`, …) and **provider functions** (`provider::<p>::<f>`) are classified `external`. With `--include-external` they become `<external>::name` nodes (fixture a).
- **Not applicable**: `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, rust_types-like tables, implicit packages.

## 14. Confidence & certainty

- **`high`** is reachable only through the generic intra-file linker: a same-file block whose simple name equals the called function name. As shown in 07 this is always a **false** edge (fixture c: three `high` `intra-file` edges to `resource::null_resource::myfn`). No other path yields `high`.
- **`medium`**: none observed. Cross-file never binds (04).
- **`low`**: only synthetic exit-node edges (`--include-stdlib` / `--include-external`; fixtures a and c).
- **Mermaid `?` marker**: does not appear on the false intra-file edges, because they are `high`. Exit-node edges carry it (generic behaviour, not separately verified for HCL).

## 15. Dynamic use & reflection

- **None.** HCL is not covered by `dynuse` (Python/JS/Java/C#/Ruby/PHP per CLAUDE.md; no `hcl` in `crates/cgg-lang/src/plugins/dynuse.rs` per grep), and `signals().dyn_uses` is false.
- `lookup()`, `try()` and `can()` are ordinary stdlib calls. Dynamic resource addressing (`for_each`, `count`) is not modelled.

## 16. Known gaps & limitations

- **References are not edges.** Traversals (`var.*`, `local.*`, `module.*`, `data.*`, `<type>.<name>.*`) and `depends_on` are ignored, so the Terraform dependency graph, which is the main use of a Terraform "call graph", is absent (fixture a, `hcl.rs:56-67`).
- **False `high` intra-file edges**: a function call binds to a same-file block whose last label matches (fixture c). This contradicts the definition of `high` as structurally certain.
- **No cross-file resolution at all.** Module `source` paths, Terragrunt `include`/`dependency` and file functions are not followed (04; `CHANGELOG.md:5471`; `skills/cgg/SKILL.md:501`).
- **No grammar-error recovery**: a file with a parse error yields no callables (fixture d, `broken.tf`).
- **Extensions not recognised**: `.tf.json`, `.tofu`, `.nomad`, `.tftpl` (fixture d).
- **Flat qualified names**: nested blocks lose their parent. `dynamic::rule` does not say which resource it belongs to, and Nomad `group`/`task` drop the `job` (fixture a). Root and child module blocks with the same labels are indistinguishable by QN.
- **Only blocks with ≥2 labels are callables**:
  - `locals`, `terraform`, `moved`, `import`, `removed` and Packer `build` are invisible.
  - Calls inside top-level `locals` and in `.tfvars` have `src: null` (fixture a audit).
- **The stdlib list lags Terraform 1.5–1.10**: `strcontains`, `issensitive`, `templatestring`, `ephemeralasnull` are missing.
- **Performance history**: `hcl-terraform-aws` (mostly Go) repeatedly timed out or was excluded in corpus runs (`CHANGELOG.md:794,919,1221,1237,1356`), and a reverted index left "`hcl` is 49s instead of 17s" (`CHANGELOG.md:3895-3900`). These concern the Go side, not the HCL plugin.
- **Unit tests are minimal**: three tests in `hcl.rs:162-187` (`plugin_loads`, `extracts_blocks`, `extracts_module_blocks`), which assert only that definitions are non-empty. There are no integration tests or fixtures under `crates/*/tests` for HCL; the only mention is the skip in `detect_prefixes.rs`.
- **Benchmark**: `terraform-vpc` reports `1,779 | 0 | —` (callables, edges, cross-file; `README.md:1331`). Zero edges in a real module confirms the extraction gap.
- No ignored tests and no `gap_for()` entry for HCL (grep).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.tf`/`.hcl`/`.tfvars` only; no `.tf.json`/`.tofu`/`.nomad`; no error recovery |
| 02 Callables extracted | minimal | Blocks with ≥2 labels as flat FreeFunctions; no nesting in QN; locals/tfvars invisible |
| 03 Call-site forms | partial | All `function_call`s in any expression; no traversals/`depends_on`/module source |
| 04 Imports & module resolution | none | No imports recorded; cross-file never binds (`candidates-in-other-files`) |
| 05 Type inference & receiver typing | n/a | No receivers; variable `type` not read |
| 06 Inheritance, dispatch & overloading | n/a | Language has none; not in any dispatch/arity table |
| 07 Scoping & shadowing | none | No locals; same-file block label shadows function name, giving false high edges |
| 08 Macros, preprocessor & codegen | none | `dynamic` blocks are plain callables; templates not parsed |
| 09 FFI & cross-language links | none | No provider (Go) linkage, not a descriptor language |
| 10 Frameworks & entry points | none | Zero rules; coverage reports `no_markers` |
| 11 Test harness entries | none | No `terraform test` / `.tftest.hcl` rule |
| 12 Dead-code analysis | minimal | ScriptDriven class: root-dependent findings withheld, capped Low |
| 13 Builtins & stdlib tables | partial | 125-name `hcl.txt`; misses recent Terraform functions; Terragrunt/provider fns external |
| 14 Confidence & certainty | minimal | Only `high` path is a false intra-file label match; exit edges `low` |
| 15 Dynamic use & reflection | none | Not covered by dynuse |
| 16 Known gaps & limitations | n/a | References-not-edges, false high edges, no cross-file, flat QNs, thin tests |
