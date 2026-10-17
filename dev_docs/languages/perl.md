# perl

The Perl plugin (`PerlPlugin`, `crates/cgg-lang/src/plugins/perl.rs`, 1,030 lines, of which 20 are inline tests at perl.rs:717-1030) parses Perl 5 with the crates.io grammar `tree-sitter-perl`. The workspace pins `"1.1"` (Cargo.toml:112), which Cargo.lock:1664-1666 resolves to 1.1.2. The plugin binds `tree_sitter_perl::LANGUAGE` (perl.rs:27-29); nothing is vendored. It does not override `resolver_kind`, `max_line_bytes` or `ts_language_for`.

A single recursive `PerlWalker` (perl.rs:52-704) does five things:

- It keeps a running `package` scope.
- It records every named `sub` as a `FreeFunction`.
- It records `use`/`require` imports, package declarations (`perl-package`) and base classes (`perl-isa`).
- It records bareword calls and `->` method calls. A method call is marked `invocant` when its receiver is the sub's own invocant.
- It captures registrar arguments (Mojolicious/Dancer2 routes), and mints `handler_at_<line>` closures for inline route handlers.

`signals()` declares only `value_refs: true` (perl.rs:21-26). Visibility, attributes, exports, test_defs, dyn_uses, unreachable and impls are all false.

Perl-specific resolution lives in a few places:

- `libscope.rs`: `build_perl`/`resolve_perl`, at libscope.rs:1148-1235. This is the invocant/`@ISA` walk.
- `intra_file.rs:329-350`: a bare call stays in its own package.
- `cross_file.rs`:
  - the `"perl"` arm of `bare_call_reaches` (4159-4162);
  - `perl_package` (5112-5117);
  - the `resolve_perl` hook (5476-5498).

The dead-code pass classifies Perl as `SCRIPT_DRIVEN` (`crates/cgg-resolve/src/deadcode/caps.rs:27-36`).

All fixtures cited below live under `<scratch>/parity/perl/` (f1 … f11). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`. Results were dumped with `show.py` (graph) and `aud.py` (audit sidecar) in that directory.

## 01. Detection & parsing

**Supported:**

- **Extensions `.pl`, `.pm`, `.t`** (perl.rs:15-17). Matching is case-insensitive: in f6, `Makefile.PL` → `extension:.pl` and `X.PM` → `extension:.pm`.
- **Shebang needle `perl`** (perl.rs:18-20). Detection is a substring test over the whole first line, with plugins iterated in registration order (detect.rs:63-75; `read_shebang` reads ≤256 bytes, detect.rs:207-220). Perl is registered 24th (plugins.rs:148). In f6 these all report `shebang:perl`:
  - `#!/usr/bin/env perl` (extensionless `script/noext`);
  - `#!/usr/local/bin/perl5.36 -w`;
  - `#!perl` in `t.t`.
- **POD, `__END__`, `__DATA__`**: the grammar handles them. f6 `pod.pl` extracts `pod_around` and `after_pod` but not `in_pod`, which sits inside `=pod … =cut`. f6 `end.pl`/`data.pl` extract nothing after `__END__`/`__DATA__`.
- **Error-tolerant parse**: a file with syntax errors is extracted as-is with `parse_status: error`, and definitions after the error are still captured. For example, f2 `a.pl` errored and still produced all 16 subs. Test `malformed_source_does_not_panic` (perl.rs:954-958).

**Not supported or partial:**

- **Shebang hijack by earlier plugins**: `#!/home/shared/bin/perl` is detected as **bash** (`shebang:sh`, f6 `shared.pl`). Bash (registered 12th, plugins.rs:135) has needle `sh`, and `shared` contains it. Any interpreter path containing `sh` misroutes a Perl script.
- **Unrecognised extensions** (f6, all `file_skipped … unknown-extension`):
  - `.cgi`, `.psgi` (so `app.psgi`, the PSGI app entry, is never parsed: f5, f6);
  - `.plx`, `.pod`;
  - `.xs` (unverified; no plugin lists it).
- **Built-in deny dir `bin`**: a Perl tree's `bin/` scripts are skipped entirely (`file_skipped … builtin, detail "bin"`, f1 `bin/app.pl`). `BUILTIN_DENY_DIRS` is at `crates/cgg-walk/src/lib.rs:49-57`. This is not Perl-specific, but `bin/` is the conventional home of Perl entry scripts. `script/` is not denied (f5 `script/run.pl` analyzed).
- **Build-output dirs are walked**: `blib/` (MakeMaker build output, a copy of `lib/`) and Carton's `local/` are both analyzed (f6: `blib/lib/B.pm` and `local/lib/L.pm` both produced callables). This can double every module.
- **Grammar parse failures on legal Perl**: each of these gives `parse_status: error`.
  - Catalyst/attribute syntax `sub ctrl :Path('/x') :Args(0) { 1 }` (f3 `l4.pl`). `sub with_attr :lvalue` parsed in f2, but the file also errored elsewhere.
  - Glob assignment `*{"main::dyn_$_"} = sub { 1 } for qw(a b);` (f3 `l2.pl`).
  - Fully qualified sub names `sub Other::Pkg::qualified_def { 1 }` (f10).
  - Perl 5.38 `class`/`field`/`method` (f4 `Modern.pm`).
- **No re-parse/fallback** strategy, unlike C#'s `#if` blanking.
- **Depth / minified handling**: only the shared `MAX_TREE_DEPTH` (4,000) → `SkipReason::TooDeep` (CLAUDE.md, `cgg/src/lib.rs`). There is no `max_line_bytes` override and no locals, so the 2,000-byte locals rule does not apply.

## 02. Callables extracted

Every definition is recorded by `record_def` (perl.rs:302-321) as `DefVariant::FreeFunction` (kind `function` in JSON). `visibility` is always `""` and `attributes` always empty (perl.rs:317-318). `signature_hint` is `extract_signature(text)`, which is the text up to the body. In f2 it kept `sub with_proto($$)`, `sub with_sig ($a, $b = 2, @rest)`, `sub ctrl :Path('/x') :Args(0)` and `my sub lexical_sub`.

**Qualified names** are `<package>::<name>`. The package is a running scope set by each `package` statement (perl.rs:78-106; `qn()` perl.rs:68-74). A sub before any `package` has a bare QN (`a1`, f6). After an explicit `package main;` it is `main::zlist` (f2). Tests: `package_scope` (perl.rs:763-772) and `a_second_package_rescopes_later_subs` (perl.rs:843-851).

**Supported shapes:**

- `sub name { … }` → `Pkg::name` (test `subroutine_captured`, perl.rs:737-742).
- Prototype `sub f($$) {…}`, signature `sub f ($a, $b = 2, @rest) {…}`, attribute `sub f :lvalue {…}` → one callable each (f2 L28-30).
- `my sub lexical_sub {…}` and `our sub our_sub {…}` → recorded as package subs `main::lexical_sub` and `main::our_sub` (f2 L32-33). The lexical scope of `my sub` is not modelled.
- Forward declaration `sub forward_decl;` (no body) → **is a callable** `main::forward_decl` (f2 L34). The grammar's `function_definition` accepts `semi_colon` in place of the body. The result is a phantom node that can surface as dead code.
- Special subs `AUTOLOAD` and `DESTROY` → ordinary callables (f2 L39-40). They get no special rooting (§12).
- A named sub nested inside another, `sub outer { sub nested_named {…} nested_named(); }`, → flat `main::nested_named`. This matches Perl semantics: named subs are package-global. The call binds `outer -> nested_named` high (f2 L46).
- Redefinitions `sub dup {1} sub dup {2}` → two nodes with the same QN (f10). A bare call to them is `ambiguous-in-file` (§14).
- **Synthetic inline route handlers**: a `sub { … }` argument of a registrar call that also has a leading route string becomes a `DefVariant::NamedClosure` named `handler_at_<line>` with attribute `synthetic` (perl.rs:658-703). Examples: `get '/x' => sub {…}`, `$r->any('/inline' => sub {…})`. Evidence: f5 (`Dapp::handler_at_3`, `MyApp::handler_at_9`) and f9 (`handler_at_2`, `handler_at_6`). Tests `an_inline_sub_handler_gets_a_name` (perl.rs:885-906) and `an_ordinary_callback_mints_no_handler` (perl.rs:908-920).

**Not supported:**

- **Anonymous subs** `my $f = sub {…}`, `{ key => sub {…} }`, callbacks to `hook`/`helper`, Moose `around`/`before`, `subtest 'x' => sub {…}`: no callable. Calls inside them are attributed to the enclosing named sub, or are `no-enclosing-callable` at top level (f2 L44, f4 L9-10, f5 L10-11).
- **BEGIN/END/INIT/CHECK blocks**: no callable. Calls inside are `no-enclosing-callable` (f2 L37).
- **Top-level script code**: no file-level callable. Calls are `no-enclosing-callable` (f2 L41-43). This is what makes Perl `SCRIPT_DRIVEN` (§12).
- **Fully qualified definition** `sub Other::Pkg::qualified_def {…}`: mis-named `Blk::Other` (the package-prefix fragment under the current scope), and the file errors (f10).
- **Block packages** `package Blk { … }`: the scope is not popped at the block's end. `sub after_block` written after the block is recorded as `Blk::after_block` instead of `Outer::after_block` (f10). `package_statement` handling clears and pushes but never restores (perl.rs:94-96).
- **`package Name VERSION;`**: handled (`Outer::o1`, f10).
- **Method::Signatures / Function::Parameters `method name {…}` / `func name {…}`**: the grammar parses these as `call_expression_with_sub` wrapping `function_definition_without_sub`, and the walker matches neither (perl.rs:107, 161-164). No callable `hello`/`world` (f4 `Modern.pm`).
- **Perl 5.38 `class`/`field`/`method`** (Corinna): not parsed. `class`, `Point`, `field` and `package` come out as external bareword calls (f4).
- **Moose/Moo `has` accessors** (`has radius => (is => 'rw', builder => '_build_radius')`): no accessor callable. `$self->radius` goes to the external bucket (f4 L6). The `builder`/`trigger`/`default` subs named in strings are never linked (f4 `_build_radius` has no caller). CHANGELOG.md:705-706 lists "Perl Moose/Moo `has` accessors" as not covered.
- **Glob-assigned subs** `*alias_glob = \&zlist;`, `*{"name"} = sub {…}`: no callable. A later `alias_glob()` call is external (f2 L43).
- **`eval 'sub cond_def {…}'`** (string-defined subs): nothing (f10).
- **`use constant PI => …`**: no callable. A use of `PI` is an external bareword call (f2 L21). The grammar's `use_constant_statement` is not matched by the walker.
- **Owners / methods**: Perl has no class construct cgg models. Every sub is a `FreeFunction` whose "owner" is the package prefix of its QN. No `InherentMethod`/`TraitMethod` variant is ever produced.
- **Abstract members**: not applicable as a declaration form. The Moose `requires 'area'` role requirement is just an external call to `requires` (f4 `Role/Draw.pm`).
- **Overloaded operators** (`use overload '+' => \&add`): no operator callables. `\&add` becomes a direct call edge from whichever sub contains it, or a `no-enclosing-callable` at top level (unverified as a fixture; inferred from the `\&f` behaviour in §03).

## 03. Call-site forms

The walker records a `RefRecord` for four node kinds:

- `call_expression_with_bareword`, `call_expression_with_args_with_brackets` and `call_expression_with_spaced_args` go through `record_call` (perl.rs:161-180, 364-396). The callee name is the text of the node's first child.
- `method_invocation` goes through `record_method_call` (perl.rs:181-185, 398-452).

`call_expression` and `method_call` are also listed, but this grammar version produces neither node kind (checked against `node-types.json` of tree-sitter-perl 1.1.2).

**Supported:**

- **Bare call** `helper(1)`, `local_amp;`, `local_amp 1, 2;`, `&local_amp;`, `&local_amp();` → bare `RefRecord`. All five bound high intra-file in f1 (L15-18). Test `a_call_is_a_reference` (perl.rs:823-831).
- **Method call on a variable** `$obj->run(1)` → `name=run`, `receiver_hint="$obj"` (test `a_class_method_call_keeps_its_package`, perl.rs:746-761).
- **Class-method call** `Pkg->m()` → `receiver_hint="Pkg"`, taken from the `package_name` field (perl.rs:408-415; FINDINGS A6, CHANGELOG.md:460-461). It binds:
  - `B->dup_m()` → `B::dup_m` high (f7 L12);
  - `My::Util->util_fn(5)` high (f1 L14);
  - `Repo->zlist()` high (f2 L19).
- **Invocant calls** are marked `RefRecord::invocant = true` (perl.rs:416-421). The invocant is either:
  - the sub's signature's first scalar (`sub m3 ($s, $t)`);
  - the variable its first statement takes from `@_` (`my $self = shift;`, `shift()`, `shift @_`, `shift(@_)`, `$_[0]`, or `my ($c, %a) = @_;`; `invocant_of`, perl.rs:207-255);
  - a literal `shift->m`.

  Test `invocant_calls_and_bases_are_recorded` (perl.rs:987-1029). Resolution is described in §06.
- **`$self->SUPER::m()`** (invocant only) → `receiver_hint="SUPER"` (perl.rs:422-433). A `$x->SUPER::m` on a non-invocant keeps `$x`.
- **`__PACKAGE__->m()`** → receiver `__PACKAGE__`; bound high intra-file (f4 `Shape::describe -> Shape::area` L8).
- **Chained calls** `$r->get('/x')->to('a#b')`: each link is a `method_invocation`. The outer link's receiver hint is the whole upstream text (`JSON::PP->new` for `JSON::PP->new->encode`, f8 external exit `<external>::JSON::PP->new::encode`).
- **Fat-comma keys are not calls**: `name => $v` and `scalar @{…}, k => …` (WS14/WS16, perl.rs:365-379). Tests:
  - `a_bareword_before_a_fat_comma_is_not_a_call` (perl.rs:960-967);
  - `a_bareword_ending_a_list_operator_before_a_fat_comma_is_not_a_call` (perl.rs:969-980).
- **Code reference `\&name`**: its inner `&name` bareword is recorded as an ordinary bare call. `my $cr = \&local_amp;` produced a **direct** high edge `status -> local_amp` (f1 L19), not a `Via::Reference` edge. With `--reference-edges --dynamic-dispatch`, f1 had 0 `ref`/`dyn` edges.
- **Registrar arguments** (perl.rs:518-611): for a call whose callee, or the last segment of `recv.verb`, is a registrar verb of a Perl rule, each argument slot is captured as follows.
  - String literals → `STRING_REF_HINT` refs, except the route string itself.
  - `\&handler`, `$handler` or a bareword not followed by `=>` → `VALUE_REF_HINT` refs (`value_ref_name`, perl.rs:614-648).
  - An inline `sub {…}` → a `handler_at_N` closure.

  The verb gate is `ExtractCtx::is_registrar_verb`, scoped to the file's language (crates/cgg-lang/src/lib.rs:202-215). Perl's built-in verbs are the union of the mojolicious and dancer2 `registrars` (rules.rs:3164-3175, 3194): `get post put patch delete options any websocket under route`.

  The route comes from the call's first string, or for a Mojolicious chain from up to two upstream links (`registrar_route`, perl.rs:490-510). The context is `"$r.get"` when the receiver is simple (`is_simple_receiver`, perl.rs:709-715), else the bare verb.

  Tests:
  - `a_mojolicious_route_captures_its_handler_and_path` (perl.rs:855-869);
  - `a_chained_to_string_is_captured_with_the_upstream_path` (perl.rs:871-883);
  - `a_hash_key_is_not_a_handler` (perl.rs:922-933);
  - `a_non_registrar_call_captures_nothing` (perl.rs:935-945).

**Not supported or partial:**

- **Fully qualified function call** `Pkg::f()` / `My::Util::util_fn(4)` / `A::same_file_target()`: **never binds**, even when the target is in the same file. The ref name is the whole path `Pkg::f`, and it lands in the external bucket as `no-candidate-in-file` (f1 L13; f2 L18, L20; f7 L6 same-file; f7 L11). `intra_file.rs:336` explicitly skips names containing `::`, and no other step splits them. The intra_file comment (329-333) says another package's sub "is reached as `Repo::zlist(…)`", but in practice that call reaches nothing.
- **Spurious package ref**: for `Pkg::f(…)` the inner `call_expression_with_bareword` is walked again (perl.rs:178), and its first child is the `package_name`. A second ref named `Pkg` is therefore recorded:
  - `My::Util` (f1 L13) and `Repo` (f2 L18, L20) are unresolved `no-candidate-in-file`;
  - `<external>::Data::Dumper` is an exit node under `--include-external` (f8 L16).
- **Double-recorded bare calls**: `f(x)` and `f x` record the name twice, once for the outer node and once for the inner bareword (perl.rs:177-178). Every such site appears twice in the audit buckets (f2: `floor`, `max`, `say`, `nonexistent_fn` ×2; f1: `sort`, `map` ×2). Edges dedupe.
- **Dynamic method name** `$self->$m()` / `$self->$cr()`: the callee is a `scalar_variable`, there is no `function_name`, and nothing is recorded (f1 L24-25; f4 Dyn.pm L8).
- **Code-ref invocation** `$cr->()`, `$anon->()`, `&$cr()`, `&$cb;`, `&{"Dyn::go"}()`, `$orig->(@_)`: these are `method_invocation` with an `array` callee or `call_expression_with_variable`, and none is recorded (f1 L20-21, 23; f4 Dyn.pm L11-12).
- **`goto &sub`**: f4 Dyn.pm L13 recorded `go` as `no-enclosing-callable` plus `candidates-in-other-files`. The surrounding sub's span was cut short by a parse error; the clean case is unverified.
- **`__SUB__->(…)`** (`call_expression_recursive`): not matched (perl.rs:161-164).
- **`bless`**, `print`/`say` with filehandle forms, `sort BLOCK`, `map BLOCK`: some come through as stdlib refs (`sort`, `map`, `print`, `say`, f1/f2/f8). `bless` is its own grammar node kind and was never seen as a ref (f1 `new`).
- **Arity**: `RefRecord::arity` is never set (perl.rs:567, 589, 689: `arity: None`; default elsewhere).
- **Paren-less flag, explicit generics, call-operator, kwargs**: not applicable / never set. `kwargs` is always empty, so `name => value` pairs are not keyword arguments.
- **Value refs outside registrars**: `value_ref` is never set on a plain call. The only value-position refs come from registrar capture.

## 04. Imports & module resolution

**Records produced** (`record_import`, perl.rs:323-362; `record_bases` 259-300; package 78-106):

- `use Foo::Bar;`, `use Foo qw(a b);`, `use lib 'lib';`, `no warnings 'x';` → `ImportRecord{kind:"use", path:"Foo::Bar", alias:"Bar"}`. The alias is the last `::` segment (test `an_import_alias_is_the_last_path_segment`, perl.rs:816-821). The path comes from direct `package_name`/`bareword`/`string` children.
- `require Foo::Bar;` → `kind:"require"` (test `use_and_require_keep_their_own_kinds`, perl.rs:784-800).
- `use strict`/`use warnings` (any path starting with those words) are skipped (perl.rs:340-341; test `pragmas_are_not_imports`, perl.rs:802-814).
- `package X;` → `kind:"perl-package", path:"X"`.
- Base declarations → `kind:"perl-isa", path:<base>, alias:<current package>` (§06).

**How the resolver uses them** (`cross_file.rs` import loop):

- **`use M`** → the `"use"` arm pushes `M` onto `unqualified_prefixes` (cross_file.rs:1940-1955). A bare call may then reach any sub of package `M`, because `via_import` satisfies the Perl branch of `bare_call_reaches` (cross_file.rs:4159-4162). Examples:
  - `use My::Util qw(util_fn); util_fn(3)` → `My::Util::util_fn` high `cross-file:imports` (f1 L12).
  - **Exporter lists are not read**: `use A; not_exported();` binds `A::not_exported` high, though A has no `@EXPORT` at all (f7 L9).
- **`require M`**: the `"require"` arm tries file paths `M`, `M.rb`, `M.lua`, `M.clj` and dotted `.lua`/`.clj` variants (cross_file.rs:1846-1865). It never tries `M.pm` or `M → M/…pm`, so for Perl a `require` contributes nothing to bare-name scope. `require B; B->dup_m()` still bound high (f7 L12), because the class-method lookup is by package name, not by import.
- **The generic `"import"` arm lists `"perl"`** (cross_file.rs:1679-1690), but the Perl plugin never emits kind `"import"`. That entry is dead.
- **Package membership** is per QN (`perl_package`, cross_file.rs:5112-5117). A bare call reaches only subs of the caller's own package (`main` ≡ no package) or of a `use`d package (§07).
- **Framework detection** reads every import path regardless of kind, including `perl-isa` (frameworks/mod.rs:263-270). For example, `use base 'Catalyst::Controller'` made catalyst "seen" (f5 `Cat.pm`).

**Not supported:**

- **Module → file mapping**: there is no `FileFacts::packages`, no `@INC`/`use lib` handling, and no `lib/Foo/Bar.pm` ↔ `Foo::Bar` mapping. Resolution is by package name embedded in QNs, wherever the file sits.
- **`heads.rs` external-module refusal**: Perl is not one of its languages (`grep perl heads.rs` is empty). `modules.rs` (`not-in-imported-module`) also has no Perl. An external `Foo::bar()` or bareword simply has no candidate and lands in the external bucket by name screen (f2 `floor`, `POSIX::floor`).
- **Import lists / Exporter tags** (`qw(a b)`, `:all`, `@EXPORT_OK`): not parsed.
- **`use parent -norequire, 'X'`** (`use_parent_statement`) records only `perl-isa`, never a `use` import (perl.rs:121-125). That does not matter for detection, since `perl-isa` paths are read too.
- **`use if COND, 'Mod'`, `use constant`, `use feature`, `use VERSION`** (`use_no_if_statement`, `use_constant_statement`, `use_no_feature_statement`, `use_no_version`): not matched by the walker (perl.rs:148), so no record. f2 parsed `use if …` (f3 `l1.pl` ok) but recorded nothing (code read).
- **String / dynamic requires** `require "lib/My/Util.pm"`, `eval "require $mod"`, `Module::Runtime::require_module($mod)`: not modelled. The string form records a `require` with the quoted path (perl.rs:334-338), which matches no file. `require_module` is an external call (f4).
- **Manifest reading** (`cpanfile`, `Makefile.PL`, `META.json`, `dist.ini`): none.

## 05. Type inference & receiver typing

- **None for Perl.** The plugin records no `local_types`, no `scoped_types`, no `typed_receiver`, no `arg_type` and no return types. `perl` appears in `type_hints.rs` only in the profiling tally table `lang_tallies_table!` (type_hints.rs:336-353), which has no semantic effect.
- `my $o = My::Domain->new(...); $o->greet()` → **unresolved** `candidates-in-other-files {candidates: 2}` (f1 L11). `my $h = A->new; $h->uniq_m;` with a single candidate → unresolved `candidates-in-other-files {1}` (f7 L14).
- **Untyped receivers are never fanned out by name**: step 5 (duck-typed fan-out) requires the receiver hint to start with a lowercase letter (`rh.chars().next().is_some_and(|c| c.is_lowercase())`, cross_file.rs:6861). Every Perl variable receiver starts with `$`, so `$x->m()` gets no by-name binding even with one candidate (f7 L7-8, L14). The only receivers that bind are:
  - the invocant (§06);
  - a class name (`Pkg->m`);
  - `__PACKAGE__`;
  - receivers the intra-file pass matches.

  `ref($self)->area()` bound `Shape::area` at medium via `cross-file:imports` (f4 L8).
- **Casts/constructions** (`CAST_INTRINSICS`, `CONSTRUCTING_METHODS`): no Perl entries (stdlib.rs:35-46, 58; `CONSTRUCTING_METHODS` is Ruby-only). `Pkg->new` is not read as a construction, and `bless` is not recorded.
- **`declared_type` wrappers/nullability, closure parameter typing, implicit receivers (`settle_implicit_receivers`), weak receiver types**: not applicable / never set for Perl.
- **Invocant identification** is the one receiver fact Perl has. It is syntactic (signature or first-statement `@_` unpacking), not a type.

## 06. Inheritance, dispatch & overloading

**Base records** (`perl-isa`, perl.rs:259-300). Each listed package name becomes one record. `-flags` are skipped, as are strings that are not `[A-Za-z0-9_:]`. Records are in source order. The sources are:

- `use parent -norequire, 'Base';` (`use_parent_statement`, perl.rs:121-125);
- `use base qw(A B);` and `use Mojo::Base 'Base';`, where the module name is in `PERL_BASE_PRAGMAS = ["parent","base","Mojo::Base"]` (stdlib.rs:11-15; perl.rs:151-157);
- `our @ISA = ('Base');` (perl.rs:126-142);
- Moose/Moo/Mouse `extends 'Base';` and `with 'Role';`, keywords from `PERL_BASE_KEYWORDS = ["extends","with"]` (stdlib.rs:17-20; perl.rs:165-176). Roles are treated as bases.

Test: `invocant_calls_and_bases_are_recorded` (perl.rs:987-1029).

**Invocant method resolution** (`LibIndex::resolve_perl`, libscope.rs:1186-1235, called from cross_file.rs:5476-5498 before the generic steps):

- `$self->m()` looks in the caller's package first, then its bases depth-first in declaration order (Perl's default DFS MRO), visited-guarded, capped at 64 steps.
- If found, it binds with `certain = (one cid)` → high.
- If not found while the chain passes a package no `package` statement declares, the call is **external** (e.g. `Ext::Base`).
- Otherwise it returns `None` and falls through to the generic steps.

Evidence: unit test `perl_invocant_call_reaches_the_package_and_its_bases` (cross_file.rs:13900-13955), plus these fixtures:

- f1: `$self->base_only()` → `My::Domain::base_only` high (inherited via `use parent`); `shift->init` → `My::Domain::init` high.
- f4: `$self->area()` in `Shape::Circle` → `Shape::area` high (via `extends`); `$self->render()` → `Role::Draw::render` high (via `with`).

CHANGELOG.md:681-686 describes the same behaviour.

**`$self->SUPER::m()`** starts at the caller package's bases (libscope.rs:1203-1205). f1: `$self->SUPER::greet()` in `My::Domain::Void::status` and in `My::Domain::Void::greet` → `My::Domain::greet` high.

**Not supported or partial:**

- `$class->SUPER::new(@_)` in a package with **no recorded base** returns `None` from `self.perl_isa.get(pkg)?` and lands in the external bucket (f4 `Shape::new_from`, `new 'SUPER'`). `use Moose`/`use Moo` imply `Moose::Object` as a base, and that is not recorded.
- A method on a non-invocant object (`$o->greet()`, `$obj->m`) never walks bases and never fans out (§05).
- `mro 'c3'`, `next::method`, `maybe::next::method`: not modelled (no code path; unverified as a fixture).
- `push @ISA, 'X'`, `@ISA = qw(...)` without `our`, `use base` inside a BEGIN block: only an `=` assignment whose LHS text is `@ISA`/`our @ISA` is read (perl.rs:127-134). `push` is not (code read).
- `--dynamic-dispatch` / `dispatch::inheritance_fanout`: Perl is **not** in `INHERITANCE_LANGS` (dispatch.rs:110-123). No base-member → override edges, so a call landing on `My::Domain::greet` never reaches `My::Domain::Void::greet`. f1 with `--dynamic-dispatch` had 0 `dyn` edges.
- `trait_impl_target`, `impls` signal: not set.
- `member_of_calling_class`, `overrides_below`, extension methods, `one_declaring_ancestor`: none have a Perl branch.
- **Overloading by arity**: not applicable to Perl. It is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583), and arity is never recorded.

## 07. Scoping & shadowing

**Supported:**

- **Package scoping of bare calls**, both intra-file (intra_file.rs:329-350) and cross-file (cross_file.rs:4159-4162). A bare `zlist()` in `package main` binds `main::zlist`, never the `Repo::zlist` higher in the same file. Evidence:
  - test `a_perl_bare_call_stays_in_its_package` (intra_file.rs:1500-1530);
  - f2 L17 `caller_main -> main::zlist` high;
  - CHANGELOG.md:448-461.

  A sub with no package and one in `package main` count as the same package (`perl_package` maps `main` to `""`).
- **Running package scope**: each `package` statement switches the scope for every following sub (test `a_second_package_rescopes_later_subs`).

**Not supported:**

- **`FileFacts::locals`**: Perl is not in `locals.rs`, so `shadowed-by-local` never fires. `my $x` variables are not tracked.
- **Lexical narrowing of nested definitions**: Perl is not in `lexically_scoped` (intra_file.rs:119-135). That is correct for named subs, which are package-global. It is wrong for `my sub`, which is recorded as a package sub (§02).
- **Positional shadowing**: none.
- **Block package scope**: `package X { … }` leaks to the rest of the file (§02, f10).
- **Conditional definitions** (`DefRecord::conditional`): never set. `if (...) { *f = sub {…} }` defines nothing, and two `sub dup` copies make the call `ambiguous-in-file` (f10).
- **`local`/`our` dynamic scoping, `no strict 'refs'` symbol-table manipulation**: not modelled.

## 08. Macros, preprocessor & codegen

- **Not applicable** for macro expansion: Perl has no `#define` or `macro_rules!`.
- **Source filters** (`use Filter::Simple`, `Switch`), **string `eval` codegen** (`eval "sub $name {…}"`), **`AUTOLOAD`-generated methods**, **`*{"Pkg::$name"} = sub {…}` glob generation**, **Moose `has`-generated accessors** and **`Class::Accessor`/`Mojo::Base` `has`**: none are modelled. They define no callable and carry no `macro:<name>` tag.
- **`name_mentions`**: never populated by the Perl plugin (no assignment in perl.rs). The string names in `builder => '_build_radius'`, `->can('area')` or `->to('users#list')` are therefore not doubt for the dead-code report. `->to('users#list')` is recorded only as a registrar `STRING_REF_HINT`.
- **POD**: handled by the grammar (§01). Not codegen.

## 09. FFI & cross-language links

- **XS / `Inline::C` / `FFI::Platypus` / SWIG**: no Perl coverage in `ffi.rs` (`grep perl crates/cgg-resolve/src/ffi.rs` is empty). `.xs` files are not detected (§01).
- **Descriptor links** (protobuf rpc → host method): `descriptor.rs` has no Perl branch.
- **Markup code-behind**: not applicable.

## 10. Frameworks & entry points

Every rule in `crates/cgg-core/src/frameworks/rules.rs` with `language: "perl"`; no multi-language rule includes Perl. `detect` prefixes are matched against every import path of a Perl file. Activation is tree-wide once any file matches (frameworks/mod.rs:58-110, 250-330).

| rule id | kind | trigger | enumerates? | evidence |
| --- | --- | --- | --- | --- |
| `mojolicious` | network | detect `Mojolicious`, `Mojo::Base`; registrars `get post put patch delete options any websocket under route` | yes: registrar calls (bare `get '/x' => …` and `$r->get(…)`) | rules.rs:3158-3184; f5, f9 |
| `dancer2` | network | detect `Dancer2`, `Dancer`; registrars `get post put patch delete options any` | yes: registrar calls | rules.rs:3185-3201; f5 |
| `catalyst` | network | detect `Catalyst` | **no**: declared gap (`gap` text at rules.rs:3216) | rules.rs:3202-3218; f5 "seen, no rules" |
| `plack` | network | detect `Plack` | **no**: declared gap (rules.rs:3233) | rules.rs:3219-3235 |
| `test-more` | test | detect `Test::More`, `Test::Mojo` | **no**: declared gap (rules.rs:3250) | rules.rs:3236-3252; f5 "seen, no rules" |

**What enumeration produces** (f5, f9):

- One `<framework-entry>::network::<rule>::<verb>("<route>")` node per route, with an `entry` edge at **low** confidence. The edge goes to one of:
  - a named handler (`\&list_users`, `\&save`, `\&handler`);
  - the synthetic `handler_at_N` closure.
- `websocket '/ws' => sub {…}` → entry (f9).
- The benchmark corpus lists `perl-dancer2` (`dancer2,~plack,~test-more`) and `perl-mojolicious` (`mojolicious,~test-more`) in `APPS` (scripts/benchmark.sh:225-226), plus apps `app-perlmaven-dancer2` and `app-ravada-mojolicious` (benchmark.sh:427-429). CHANGELOG.md:5054-5057 reports 283 Mojolicious routes on its own tree.

**Not supported or partial:**

- **Cross-framework bleed**: once both mojolicious and dancer2 are active anywhere in the tree, *every* Perl registrar call is enumerated under both rules. f5 minted 8 nodes for 4 routes: `dancer2::get("/users")` for a Mojolicious `$r->get`, and `mojolicious::get("/")` for a Dancer2 `get`. Registrar matching is per language, not per file.
- **String targets**: `string_targets: false` for both rules. `$r->get('/logout')->to('users#logout')` and `->to(controller => 'Users', action => 'create')` do not link to `MyApp::Controller::Users::logout`/`create` (f5: both listed as dead findings). `get '/n' => 'named_tpl'` mints nothing (f9).
- **Route without a leading string**: `under sub {…}` (f9) and Dancer2 `any ['get','post'] => '/both' => sub {…}` (f5) mint no entry. The second has an array as its first argument, so `first_string` is the array.
- **Lifecycle methods**: `sub startup` of a Mojolicious app, `register` of a plugin, Catalyst actions, `Plack` `call`/`to_app`, and `app->start` are not rooted. In f5 `MyApp::startup` is a dead finding.
- **Mojolicious hooks/helpers** `$app->hook(before_dispatch => sub {…})`, `$app->helper(name => sub {…})`: deliberately mint nothing (test `an_ordinary_callback_mints_no_handler`).
- **Catalyst subroutine attributes** (`:Path`, `:Local`, `:Chained`, `:Args`): the plugin records no attributes (perl.rs:318), and the attribute syntax can fail to parse (f3 `l4.pl`). The `catalyst` gap prose claims `use parent -norequire, 'Catalyst::Controller'` records no import, so the controller is not seen. That claim is **stale**: the `perl-isa` record carries the path, and f5's `use base 'Catalyst::Controller'` was detected.
- **PSGI**: `app.psgi` is not even parsed (unknown extension, f5/f6).
- **No `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for` or `self_module_markers_for` entries for Perl** (rules.rs helper tables after line 11627 contain no `perl`/rule-id match: `sed -n 11627,12100p rules.rs | grep -i 'perl\|mojolicious\|dancer\|catalyst\|plack\|test-more'` is empty).
- **Other Perl frameworks with no rule at all**: Moose/Moo/Mouse (OO, not entries), Mojolicious::Plugin, Dancer (v1, detected only via the `Dancer` prefix of dancer2), CGI.pm, Web::Simple, Kelp, Poet/Mason, Template Toolkit, AnyEvent/IO::Async/POE event loops, Minion jobs, App::Cmd/MooseX::Getopt CLIs. Their imports are not even "seen" (no rule ids; code read of rules.rs perl entries).

## 11. Test harness entries

- **`test-more` (kind test) is a declared gap rule**: detected by `use Test::More` / `use Test::Mojo` and reported "seen, no rules", but no harness node is minted (rules.rs:3236-3252; f5).
  - `ok(…)`, `is(…)`, `subtest 'x' => sub {…}` and `$t->get_ok('/users')` produce no test case.
  - `subtest` bodies are anonymous (§02).
  - Top-level `.t` code has no callable.
- **Test-name rule**: `is_test_name`'s default arm (`name.starts_with("test_") || name.starts_with("Test")`, roots.rs:975-987) applies to Perl. A sub named `test_*` or `Test*` becomes a `builtin:test-name` test root anywhere, not only in test files.
- **Test-file classification**: Perl has no `by_name` rule in `classify_test_file` (testfile.rs:76-120). `TEST_DIRS` (testfile.rs:36-46) contains `test`/`tests` but **not `t`**, so a conventional `t/basic.t` is **not** a test file. Callees reached only from `.t` files count as production-live, and `--include-tests` gating has nothing to withhold (code read; f5 `test_helper` in `t/basic.t` was rooted as `toplevel:invocation`, not as a test).
- **Test::Class (`sub x : Test {…}`), Test2::V0, Test::Spec / Test::Spec-style `describe`/`it`**: no rules. `TEST_CLASS_MEMBERS_ARE_ROOTS` covers Python/PHP/Ruby only (CLAUDE.md), not Perl.
- **Benchmark/fuzz harnesses**: none.

## 12. Dead-code analysis

- **Language class `script-driven`**: Perl is in `SCRIPT_DRIVEN` (deadcode/caps.rs:27-36; class chosen at caps.rs:163-166). The consequences:
  - Every root-dependent finding category is suppressed with `SuppressionReason::ScriptDriven` (deadcode/mod.rs:811-812).
  - Evidence `language-is-script-driven` is attached (mod.rs:927).
  - The capability table reports `max_confidence: medium`.

  In f5 (`--dead-code --include-tests --dead-code-confidence low --dead-code-format json`) every finding was `never-referenced` at **low**, with evidence `language-is-script-driven`, `language-lacks-attributes`, `language-lacks-dispatch-model`, `language-lacks-visibility`, `no-incoming-edges`, `no-unresolved-site-with-name`. At the default (high) confidence 0 findings were shown and 7 withheld.
- **Measured signal coverage** (f5 capability row): `visibility: none`, `attributes: none`, `value_references: full` (declared via `signals().value_refs`), `dispatch: none`, `exports: none`, `test_tagging: none`.
- **Root kinds that fire for Perl**:
  - `framework:entry-node` (§10);
  - `toplevel:invocation`: any callable whose `(language, simple_name)` matches a `no-enclosing-callable`/`value-ref-no-enclosing` site anywhere (roots.rs:560-611). f5: `Dapp::save` from `post '/save' => \&save` at top level; `test_helper` from top-level `test_helper();` in `t/basic.t`. The match is by simple name, so a top-level `new()` would root every `new`. A top-level *qualified* call `MyApp::helper_fn()` does not match a simple name.
  - `builtin:test-name` for `test_*`/`Test*` subs (§11).
- **No public/exported roots**: no visibility or `@EXPORT` is read, so a library's API subs are dead unless called in-tree.
- **No special-sub roots**: `import`, `unimport`, `AUTOLOAD`, `DESTROY`, `BEGIN`, `new`, `can`, `isa`, `VERSION` and `CLONE` are not rooted (code read: no Perl names in roots.rs).
- **Phantom forward declarations**: `sub forward_decl;` is a callable and can be reported (§02).
- **Confidence caps**: none of `implements-external-contract`, `macro-generated` or `reflection` (no dynuse) applies. An inherited override (`My::Domain::Void::greet`) gets no override doubt, because Perl is not in `INHERITANCE_LANGS`.
- **Unresolved-doubt handling**: generic. An unresolved same-name site (`no-unresolved-site-with-name` / its opposite) is read by simple name. That includes the many `candidates-in-other-files` sites from untyped `$x->m()` (§05), which act as doubt for same-named subs (code read; mechanism in CLAUDE.md §dispatch).
- **Moose builders, string-dispatched controller actions, `can('m')` targets**: false-positive dead (f4 `_build_radius`; f5 `Users::logout`/`create`).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/perl.txt`**: 167 names (`wc -l`), embedded at stdlib.rs:207 and registered at stdlib.rs:248. The list covers:
  - core builtins: `print`, `push`, `shift`, `sort`, `map`, `die`, `eval`, `bless`, `ref`, `scalar`, `wantarray`, …;
  - common module exports: `List::Util` `first`/`max`/`min`/`sum`/`sum0`/`uniq`/`any`/`all`/`none`/`pair*`; `Scalar::Util` `blessed`/`weaken`/`refaddr`/`looks_like_number`; `Carp` `croak`/`carp`/`confess`; `File::Spec`/`File::Basename`/`Cwd` `catfile`/`catdir`/`basename`/`dirname`/`abs_path`/`getcwd`/`realpath`/`rel2abs`/`fileparse`; `File::Temp` `tempfile`/`tempdir`; `Time::HiRes` `gettimeofday`/`tv_interval`/`usleep`/`nanosleep`; `Data::Dumper` `Dumper`;
  - keywords/pragma words: `my`, `our`, `local`, `use`, `no`, `strict`, `warnings`, `utf8`, `STDERR`, `STDIN`, `STDOUT`, `List::Util`.
- **Behaviour**:
  - A stdlib-named call with no in-tree definition goes to the stdlib bucket.
  - `--include-stdlib` mints `<stdlib>::<name>` exit nodes at low confidence (f8: `print`, `printf`, `sprintf`, `push`, `scalar`, `join`, `open`, `die`, `croak`, `blessed`, `first`, `length`, `wantarray`).
  - A same-named sub in the file still wins: f11 defined `max`, `first`, `format` and `any`, and all four bare calls bound high intra-file.
  - Step 5 skips stdlib-named methods (cross_file.rs:6863-6866), which is moot for Perl (§05).
- **Perl-specific tables in stdlib.rs**: `PERL_BASE_PRAGMAS` (stdlib.rs:11-15) and `PERL_BASE_KEYWORDS` (stdlib.rs:17-20); see §06.
- **`CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `TOP_TYPES`, implicit packages, `rust_types`-like wrapper tables**: no Perl entries (stdlib.rs:35-46, 58, 151-155).
- **Gaps in the list** (from fixtures):
  - `floor` from `POSIX` is not stdlib; it went to the external bucket (f2 L22).
  - `bless` is listed, but the grammar's `bless` node is never recorded as a call.
  - `say` is listed (stdlib, f2 L25).

## 14. Confidence & certainty

**High:**

- **Intra-file single candidate**: `local_amp`, `inner_helper`, `__PACKAGE__->area` (f1, f4) and `main::zlist` (f2) are all high.
- **Invocant / `SUPER` / class-method lookups with exactly one sub found**: `Scoped::Bound { certain: cids.len() == 1 }` (libscope.rs:1216-1220) → `module_certain` (cross_file.rs:5482-5485) → high. Examples: f1 `base_only`, `greet`, `init`, `new`; f4 `area`, `render`; f7 `B->dup_m`.
- **Bare call reached through a `use`d package with one carrier of the name**: `util_fn` (f1), `not_exported` (f7). These are high even though Exporter semantics were not checked.

**Medium:**

- **Same simple name in several packages**: `dup_m()` (with `use A`, and B also defines `dup_m`) → `A::dup_m` medium (f7 L10). The mermaid label is `?` (f7: `N6 -->|?| N1`).
- **`ref($self)->area()`** bound medium (f4 L8).

**Low:**

- Every framework `entry` edge (f5, f9).
- Every `--include-stdlib` / `--include-external` exit edge (f8).

**Refused:**

- **Same-file redefinitions**: `ambiguous-in-file`, no edge (f10 `dup`; f2 L35 `\&zlist` at top level with two `zlist` subs in different packages).
- **Untyped receivers**: `candidates-in-other-files` (§05).

No `weak_receiver_type`, `OwnerFacts` or typed-receiver certainty paths apply.

## 15. Dynamic use & reflection

- **No dynuse coverage**: Perl is not in `REFLECTIVE` (crates/cgg-lang/src/plugins/dynuse.rs:25-46), and `signals().dyn_uses` is false. None of these produces a `DynUse`, a reflection screen or a medium cap:
  - `$obj->can('name')`;
  - `$obj->$method()`;
  - `&{"Pkg::$name"}`;
  - `__PACKAGE__->can(...)`;
  - `UNIVERSAL::can`;
  - `*{$glob}{CODE}`.

  `can` is just an external method call (f4 L8, Dyn.pm L9). `$self->$m()` records nothing (§03).
- **Module loads**: `require` is in `DYN_MODULE_LOADERS` (facts.rs:165), but that is consumed only by plugins that emit dyn uses. Perl's `require $mod`, `eval "require $mod"` and `Module::Runtime::require_module` are not screened (f4 Dyn.pm).
- **String `eval`**: `eval` is a stdlib call (f4 Dyn.pm L5-6). Code inside `eval "…"` is not parsed. Calls inside `eval { … }` blocks are walked normally (`risky()` bound high, f4 Dyn.pm L6).
- **`AUTOLOAD`**: a call to a missing method is not routed to the package's `AUTOLOAD`. `AUTOLOAD` is an ordinary unrooted callable (f2 L39).

## 16. Known gaps & limitations

From code comments, CHANGELOG and rule `gap` text:

- **Catalyst** (rules.rs:3216), **Plack/PSGI** (rules.rs:3233) and **Test::More/Test::Mojo** (rules.rs:3250) are declared gap rules (§10-§11). Part of the Catalyst gap prose is stale (§10).
- CHANGELOG.md:705-706 (WS14b "Not covered"): Perl Moose/Moo `has` accessors.
- CHANGELOG.md:1394-1397: of 291 Perl regressions, 28 were recovered. The rest are `key => $obj->m` on untyped objects, or Moose accessors.
- README.md:509 lists Perl as "use, require | — | Subs and packages". README.md:1335 has a benchmark row for `mojolicious` (1,127 callables, 1,711 edges, 50%).

Found while cataloguing (fixtures):

1. **`Pkg::func()` qualified calls never bind**, even in the same file (f1 L13, f2 L18/L20, f7 L6/L11). Each one also emits a spurious ref named after the package (`Repo`, `My::Util`, `<external>::Data::Dumper`).
2. **Untyped method calls `$x->m()` never bind**. Step 5's lowercase-first-char gate (cross_file.rs:6861) excludes every `$`-prefixed receiver. There is also no local type inference from `Pkg->new` (f1 L11, f7 L7/L8/L14).
3. **`require Foo::Bar` resolves no file**: the `require` arm tries `.rb/.lua/.clj` only (cross_file.rs:1858-1865). There is no `Foo::Bar` → `Foo/Bar.pm` mapping, and the `"import"` arm's `"perl"` entry is dead (cross_file.rs:1689).
4. **Exporter is not modelled**: `use A;` makes every sub of `A` reachable by bare name at high, exported or not (f7 L9).
5. **No subroutine attributes and no visibility**:
   - attributes are hardcoded empty (perl.rs:317-318);
   - attribute-heavy subs can fail to parse (f3 `l4.pl`);
   - so no Catalyst actions, no `Test::Class` `: Test`, no `:lvalue`.
6. **Package-block scope leaks** past `package X { … }`, and fully qualified `sub A::B::c {…}` is mis-named (f10).
7. **Anonymous subs are not callables**, except registrar handlers. This covers `$cb = sub {…}`, dispatch tables `{ key => sub {…} }`, `subtest`, Moose `around`/`before`/`after`, hooks and helpers. Code-ref invocations (`$cr->()`, `&$cr`, `$self->$m`) record nothing (f1, f4).
8. **Method::Signatures / Function::Parameters `method`/`func` and Perl 5.38 `class`/`method`/`field`** are not extracted (f4).
9. **`.psgi`, `.cgi`, `.plx`, `.xs` and the `bin/` directory are skipped**. A `#!…/shared/…/perl` shebang is detected as bash (f1, f5, f6).
10. **`t/` is not a test directory and `.t` is not a test-file name** (testfile.rs:36-46, 76-120). There are no test harness entries (§11).
11. **Cross-framework bleed** between mojolicious and dancer2 registrar rules (f5).
12. **No `--dynamic-dispatch`/inheritance fan-out** (not in `INHERITANCE_LANGS`). No dynuse, no FFI/XS, no `name_mentions`.
13. **Double-recorded bare calls** (inner `call_expression_with_bareword` walked again, perl.rs:177-178) inflate audit bucket counts.
14. **Phantom callables** from forward declarations `sub f;` (f2).
15. **`$class->SUPER::new` with Moose/Moo** (no recorded `Moose::Object` base) goes to the external bucket (f4).

No Perl test is marked `#[ignore]` (`grep -n ignore crates/cgg-lang/src/plugins/perl.rs` is empty). There is no `gap_for` text for mojolicious/dancer2: their `gap` fields are `""`, rules.rs:3180, 3197.

## Summary scorecard

| section | support | one-line note |
| --- | --- | --- |
| 01 Detection & parsing | partial | `.pl/.pm/.t` + `perl` shebang. `.psgi/.cgi/.xs` unknown, `bin/` denied, `sh`-substring shebang hijack, attribute/qualified-sub parse errors |
| 02 Callables extracted | partial | Named subs with package QNs. No anonymous subs (except route handlers), Moose accessors, `method`/`func`, Corinna, glob/eval subs. Block-package scope leaks |
| 03 Call-site forms | partial | Bare, `$obj->m`, `Pkg->m`, invocant/SUPER, registrar args. `Pkg::f()` never binds; code-ref/dynamic-method calls unrecorded; no arity |
| 04 Imports & module resolution | partial | `use` opens a package to bare calls. `require` maps to no file; no Exporter lists, `@INC`, packages/heads refusal |
| 05 Type inference & receiver typing | none | No local/return typing. `$`-receivers excluded from fan-out, so untyped method calls never bind |
| 06 Inheritance, dispatch & overloading | partial | `use parent/base`, `@ISA`, `Mojo::Base`, `extends`/`with` feed a DFS invocant/SUPER walk. No inheritance fan-out, no Moose::Object base |
| 07 Scoping & shadowing | minimal | Package-scoped bare calls only. No locals, no `my sub` scope, no conditional defs |
| 08 Macros, preprocessor & codegen | none | Source filters, string-eval subs, AUTOLOAD, generated accessors unmodelled; no `name_mentions` |
| 09 FFI & cross-language links | none | No XS/Inline/FFI::Platypus; `.xs` not detected |
| 10 Frameworks & entry points | partial | mojolicious + dancer2 enumerate routes (low). catalyst/plack are gap rules. No string targets; cross-framework bleed |
| 11 Test harness entries | none | `test-more` is a gap rule. `t/` and `.t` not classified as tests; only the `test_*` name rule |
| 12 Dead-code analysis | minimal | Script-driven class. Findings capped/suppressed, roots = framework entries + top-level invocations; no visibility/exports |
| 13 Builtins & stdlib tables | full | 167-name `perl.txt` plus base pragma/keyword tables; in-file defs win over stdlib names |
| 14 Confidence & certainty | partial | High for single-candidate intra-file, invocant and `use`d-package hits. Medium on same-name copies. Redefinitions refused |
| 15 Dynamic use & reflection | none | Not in dynuse `REFLECTIVE`. `can`, `$obj->$m`, symbolic refs, `require $mod` unscreened |
| 16 Known gaps & limitations | n/a | 15 catalogued gaps; key: qualified calls, untyped receivers, `require` mapping, attributes, test files |
