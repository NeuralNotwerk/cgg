# ruby

The Ruby plugin (`RubyPlugin`, `crates/cgg-lang/src/plugins/ruby.rs`, 613 lines including tests) is a single hand-written recursive tree walker over the `tree-sitter-ruby` 0.23 grammar (`Cargo.toml:96`, `tree_sitter_ruby::LANGUAGE`, `ruby.rs:31-33`). Nothing is vendored. It is registered in `plugins::register_all` (`crates/cgg-lang/src/plugins.rs:136`) and has no `resolver_kind` override, so it uses the default resolver path: `intra_file` → `cross_file` (with Ruby-specific arms for bare-call scope, `require`, constructor and call-operator names) → frameworks → dispatch. The plugin declares `PluginSignals { dyn_uses, impls, value_refs }` and leaves `visibility`, `attributes`, `exports`, `test_defs` and `unreachable` false (`ruby.rs:22-29`). Because visibility and attributes are both absent, the dead-code pass classes Ruby as **degraded**, which caps every Ruby finding at `medium` (confirmed in a fixture run: `"class": "degraded", "max_confidence": "medium"`).

Fixtures used for this document live under `<scratch>/parity/ruby/` (`p1` to `p8`, `rails`, `rails2`, `fw`, `tl`, `det`), and were run with `target/release/cgg` 0.11.0.

## 01. Detection & parsing

Supported:

- **Extensions** `.rb`, `.rake`, `.gemspec` (`ruby.rs:16-18`). Fixture `det`: `t.rake` was analyzed as `extension:.rake` and `x.gemspec` as `extension:.gemspec`.
- **Shebangs** `ruby` and `irb` (`ruby.rs:19-21`). Fixture `det`: an extensionless `script` starting `#!/usr/bin/env ruby` was detected as `shebang:ruby`, and `irbs` starting `#!/usr/bin/irb` as `shebang:irb`.
- **Grammar**: one grammar for every file. `ts_language_for` is not overridden and there are no dialects.
- **Generic skip conditions** apply as for every language. A tree deeper than `MAX_TREE_DEPTH` = 4000 is skipped as `SkipReason::TooDeep` (`crates/cgg/src/lib.rs:155, 451-454`). A tree containing parse errors is still extracted, and the error is recorded only as the `parse_status` (`lib.rs:457`). `max_line_bytes` is not overridden, so Ruby has no long-line skip. The `MINIFIED_LINE_BYTES` gate (`lib.rs:159, 478-484`) only affects `locals::collect`, and Ruby has no locals spec anyway (see section 07).

Not supported or partial:

- **`Gemfile`, `Rakefile`, `config.ru`, `*.erb`, `*.rbw`** are skipped as `unknown-extension` (fixture `det`). The `rake` and `rack` rule `gap` texts call this out for `Rakefile` and `config.ru` (`crates/cgg-core/src/frameworks/rules.rs:2857, 10824`). `Guardfile`, `Capfile`, `.jbuilder` and `.ru` have no entry in `extensions()`, so they are not detected either (that follows from the code but was not run as a fixture).
- **ERB templates** are not parsed, so any helper called only from a view has no caller.
- **No content detection**: nothing in `detect.rs` mentions Ruby (`grep -n ruby crates/cgg-lang/src/detect.rs` finds nothing).
- **No re-parse or fallback on a parse error**, unlike C#'s `#if` blanking.

## 02. Callables extracted

Supported (the walker is at `ruby.rs:171-264`, `record_method` at `ruby.rs:278-310`):

- **Instance method**, e.g. `class Service; def run; end; end`. This becomes `DefVariant::InherentMethod` with qn `Service::run`, where the qn is the scope stack joined by `::` (`ruby.rs:163-169`). Test `class_methods`.
- **Singleton method**, e.g. `def self.create`. This is a `DefVariant::StaticMethod` with qn `Service::create` (`ruby.rs:214-219`). Test `class_methods`. The JSON output gives it `kind: method`.
- **Constructor**, e.g. `def initialize(name)`. This becomes `DefVariant::Constructor`, rendered as `kind: constructor` (`ruby.rs:286-290`). Test `class_methods`, fixture `p1`.
- **`class << self` body**, e.g. `class << self; def singleton_m; end; end`. The `singleton_class` node has no `name`, so no scope is pushed and the method is qn `Service::singleton_m`, recorded as `InherentMethod` rather than `StaticMethod` (`ruby.rs:173-197`, fixture `p1`).
- **Module methods**, e.g. `module Util; def self.fmt(x); end; def mixed_in; end; end`. These become `Util::fmt` and `Util::mixed_in` (fixture `p1`).
- **Nested modules and classes**, e.g. `module Shop; class Base; def process`. The qn is `Shop::Base::process` (fixture `p3`).
- **Compact class name**, e.g. `class Shop::Base` at top level. The name field text `Shop::Base` is pushed verbatim, so `Shop::Base::reopened` lands on the same owner path as the nested form (fixture `p3`).
- **Top-level `def`**, e.g. `def top_helper(a)`. The qn is the bare name, still recorded as `InherentMethod` (`kind: method`). `bare_call_reaches` deliberately treats an owner-less qn as top-level (`crates/cgg-resolve/src/cross_file.rs:4146-4149`).
- **Operator, predicate, bang and setter methods**, e.g. `def ==(o)`, `def [](i)`, `def ok?`, `def save!`, `def name=(v)`, `def +(o)`. The name is taken verbatim and becomes `Service::==`, `Service::[]`, `Service::ok?`, `Service::save!`, `Service::name=` (fixture `p1`).
- **Nested `def` inside a method**, e.g. `def nested; def inner_def; end; end`. This becomes `Other::inner_def`, owned by the class, which matches Ruby semantics (fixture `p1`).
- **`method_missing` and `respond_to_missing?`** are extracted as ordinary methods (fixture `p1`).
- **Inline route handler block**, e.g. `get '/hi' do … end`. When the verb is a registrar verb of some Ruby rule and the call has a registration shape, the block becomes a `DefVariant::NamedClosure` named `handler_at_<line>` with the `synthetic` attribute (`ruby.rs:425-477`). Fixture `fw` produced `handler_at_3`, `handler_at_6` and `API::handler_at_4`.
- **`signature_hint`** is the first line of the def text via `extract_signature` (`ruby.rs:304`).
- **`base_types`** on every method is the enclosing class's superclass plus the `include`/`extend`/`prepend` constants written directly in the class body (`ruby.rs:178-185, 316-345`).

Not supported or partial:

- **`attr_accessor`/`attr_reader`/`attr_writer`** generate no callables (fixture `p1`: `attr_accessor :name` produced nothing).
- **`define_method(:x) { }`** and **`define_method("#{n}_x")`** produce no callables (fixtures `p1` and `p7`).
- **`alias_method :run2, :run` and `alias run3 run`** produce no callable and no alias link (fixture `p1`).
- **Blocks, procs and lambdas** (`lambda { |v| }`, `-> {}`, `do … end`) are not callables, except for the registrar inline handlers above. Their bodies belong to the enclosing method, or to no callable at top level.
- **`Struct.new(:x) do def norm; end end` and `Class.new do def anon_m; end end`** give owner-less qns (`norm`, `anon_m`), and the constant they are assigned to (`Point`, `Anon`) is lost (fixture `p7`).
- **Compact class name inside a module.** `module Shop; class Shop::Nested` gives qn `Shop::Shop::Nested::n`, doubling the prefix: the scope stack pushes the full text and does not resolve it against the enclosing scope (fixture `p3`). This is a bug.
- **Visibility** is never recorded: `visibility: String::new()` is hard-coded (`ruby.rs:305`). `private`, `protected def`, `private_class_method` and `module_function` are ignored (fixture `p1`).
- **Attributes** are always empty for real methods (`ruby.rs:306`).
- **No `FileFacts::classes` / `ClassDecl` records.** A class's bases are known only through its methods' `base_types`, so a class with no methods (`class Empty < Base; end`) has no recorded bases (fixture `p3`, see section 06).
- **Redefinition** (`def dup_m(a)` followed by `def dup_m(a, b)`) gives two callables with the same qn. Calls bind to both at `medium`, not to the last definition (fixture `p8`).
- **Conditional definitions** (`if … def cond_m … else def cond_m`) give two callables. There is no `DefRecord::conditional` marking, so calls bind to both at `medium` (fixture `p8`).

## 03. Call-site forms

Supported (`record_call`, `ruby.rs:347-414`, and `record_bare_identifier`, `ruby.rs:146-158`):

- **Call with parentheses**, e.g. `greet('x')`. This is a `RefRecord` with an empty `receiver_hint`. Test `call_expressions`.
- **Method call on a receiver**, e.g. `obj.run`. `receiver_hint` is the receiver text, here `obj`. Test `call_expressions`.
- **Call without parentheses but with arguments**, e.g. `add 1, 2`. The grammar gives a `call` node, recorded like a parenthesised call (fixture `p4`, `Calc::run -> Calc::add` high).
- **Bare identifier with no local in scope**, e.g. `helper`. This is recorded with `paren_less: true`, and only when the name is not a local, parameter, assignment target, `rescue => e` variable or `for` variable in the current scope (`ruby.rs:82-158, 230-251`). Blocks inherit the enclosing scope, while `def`, `class` and `module` open a fresh one. Test `a_bare_identifier_that_is_no_local_is_a_paren_less_call`. The positions that count are an allowlist in `bare_call_position` (`ruby.rs:483-517`): statements, operands, arguments, interpolation, conditions, assignment RHS, call receivers, hash values, range ends and array elements. Fixture `p4`: `"v#{interp}"`, `if cond_call then` and `{ key: interp }` all bound.
- **Paren-less pruning.** `value_refs::prune_paren_less` keeps a paren-less site only when some `def` of that name declares no parameters (`crates/cgg-resolve/src/value_refs.rs:125-154`). A paren-less site that binds nothing leaves no audit record (`crates/cgg/src/lib.rs:1070-1086`; CHANGELOG lines 826-833 and 856-859).
- **Explicit `self.` call**, e.g. `self.helper2`. The receiver is `self` and it binds intra-file at high (fixture `p1`).
- **Scope-qualified call**, e.g. `Util::fmt(4)` and `Util.fmt(3)`. Both bound to `Util::fmt` at high, cross-file (fixture `p1`).
- **Construction receiver**, e.g. `Foo.new.m`, `(A::Foo.new(1)).m` or `(x).m`. `crate::receiver::peel("ruby", …)` unwraps `parenthesized_statements` (`crates/cgg-lang/src/receiver.rs:61-62`). A `call` whose method is in `CONSTRUCTING_METHODS` (`new`) on a `constant`/`scope_resolution` receiver yields that type, and the site is flagged `typed_receiver` (`receiver.rs:229-239`, `ruby.rs:389-398`). Tests `ruby_new_and_parentheses` and `crates/cgg/tests/resolve.rs::ruby_and_cpp_construction_receivers_name_their_type`. Fixture `p2`: `Dog.new(3).speak` bound high to `Dog::speak`.
- **Safe navigation**, e.g. `c&.safe_target`. Recorded as a receiver call and bound by name at `medium` (fixture `p4`).
- **Chained call**, e.g. `chain_a.chain_b`. The receiver text is `chain_a`. It bound by unique name at `medium`, not by return type (fixture `p4`).
- **Calls inside blocks**, e.g. `[1].each { |e| helper3(e) }`, are attributed to the enclosing method (fixture `p1`).
- **Registrar arguments.** For calls whose last segment is a registrar verb of a Ruby rule (`ExtractCtx::is_registrar_verb`, `crates/cgg-lang/src/lib.rs:202-217`), `registrar::capture` records string arguments with `STRING_REF_HINT`, and symbol, constant, hash or pair arguments as value references with `VALUE_REF_HINT` (`ruby.rs:411-412`, `registrar.rs:424-505`). Fixture `rails`: `before_action :load_photo`, `before_save :normalize` and `after_commit :notify, on: :create` produced `value-ref-no-enclosing` records for `load_photo`, `normalize`, `notify` and `create`.
- **`require` / `require_relative`** become import records and are never call references (`ruby.rs:359-381`).

Not supported or partial:

- **Bare `super` and `super()`** produce no reference and no edge (fixture `p3`: `Shop::Derived::step`'s `super` drew nothing). `is_super_receiver` (`cross_file.rs:3656-3664`) handles only a receiver spelled `super`.
- **Constructor calls**, e.g. `Foo.new(…)`, never reach `Foo::initialize`. `new` is a method name looked up by name, and with `--include-external` it becomes `<external>::Coll::new`, even when `Coll::initialize` exists (fixture `p6`). `constructor_names("ruby") = ["initialize"]` (`cross_file.rs:384-392`) only maps a bare `Widget(3)`-style call, which Ruby does not write. A bare `new("x")` inside `def self.build` goes to `<stdlib>::new` (fixture `p1`).
- **Setter calls**, e.g. `self.setter = 3`, are recorded as `setter`, not `setter=`, so they never reach `def setter=` (fixture `p4`, `<external>::self::setter`).
- **Operator calls**, e.g. `a + b`, `a == b` or `obj[i]`, are not recorded as calls to `def +`, `def ==` or `def []` (fixture `p4`).
- **`x.(2)` call shorthand** is dropped because the method field is empty (`ruby.rs:356-358`, fixture `p1`). **`x.call(1)`** is a method call named `call`. The call-operator mapping to `call` (`cross_file.rs:395-401`) needs `local_types`, which Ruby never records, so `x.call` went to `<external>::x::call` (fixture `p1`).
- **`&method(:name)` and `&:sym`** draw no edge and no value reference (fixture `p4`). Ruby does not call the ungated `registrar::capture_value_refs`, only the registrar-gated `capture`.
- **`method(:x)`, `send(:x)` and `public_send("x")`** draw no edge (fixture `p1`). They are captured only as dyn-uses (section 15).
- **`yield`** records nothing, which is correct because a block is not a callable.
- **`::Calc.new.add(1)`** (leading `::`) is not peeled to a type, and the site is left unresolved as `candidates-in-other-files` (fixture `p4`).
- **Arity**: `RefRecord::arity` is never set (`ruby.rs:399-406`, `ruby.rs:463`). **Keyword arguments**: `kwargs` is never filled.
- **Explicit generics**: not applicable to Ruby.

## 04. Imports & module resolution

Supported:

- **`require 'x'` and `require_relative 'x'`** (receiver-less only) give `ImportRecord { kind: "require", path }` with the quotes trimmed (`ruby.rs:359-381`). Test `require_is_import`.
- **Require resolution.** The `"require"` arm of `cross_file::resolve` (`cross_file.rs:1846-1880`) tries `path` and `path.rb` (plus the Lua and Clojure variants) relative to the requiring file through `collect_include_defs` (`cross_file.rs:3385`), with transitive depth 4. That makes the required file's definitions `direct_imports`. Fixture `p1`: `require_relative 'lib/helper'` let `top_helper(2)` bind high across files. Fixture `p5`: `require 'x/one'` from the root also produced a high edge.
- **No require needed.** Rails-style autoload works because cross-file lookup by name runs with or without a require (fixture `p2` has no `require` and `Dog.make` bound high).
- **Workspace vs external head.** A capitalised receiver that names no workspace class went to `<external>` with `--include-external` (fixture `p5`: `Nokogiri::HTML`, `Nokogiri.parse`). A name in `stdlib/ruby.txt` went to `<stdlib>` (`JSON.parse`, `puts`; fixture `p1`).

Not supported or partial:

- **A `require` does not anchor the binding to the required file.** Fixture `p5` has `top_fn` in both `lib/x/one.rb` and `lib/y/two.rb`, and `main.rb` does `require_relative 'lib/x/one'`. `top_fn` was bound at **high** to `lib/y/two.rb`, the file *not* required. Swapping the file contents gave the same result. One likely cause is that `direct_imports` maps names to qualified names, not to files (`cross_file.rs:3390`, `HashMap<String, Vec<String>>`), but that is **unverified**. Without a require, the same call fans out at medium over both copies (fixture `p5`, `c3`).
- **`heads.rs` (`WorkspaceVocab`/`FileHeads`) has no Ruby arm**, so Ruby gets no `external-module` refusal from the import table. `grep -n ruby crates/cgg-resolve/src/heads.rs` finds nothing.
- **`modules.rs` (`ModuleIndex`) has no Ruby support**, so there is no `not-in-imported-module` refusal. CHANGELOG line 1350 lists "Ruby `require`" as not covered by WS14.
- **`libscope.rs`** has no Ruby support.
- **`FileFacts::packages`** is never written. There is no Gemfile or gemspec manifest reading: `manifest.rs` is not used by `ruby.rs`.
- **`load 'x.rb'`, `autoload :X, 'x'`, `Bundler.require` and `require` with a non-literal argument** are not import records. Only the receiver-less literal `require`/`require_relative` is (`ruby.rs:360`). The padrino `gap` text (`rules.rs:2838`) notes the `Bundler.require` consequence.
- **Zeitwerk constant-to-file mapping** is not modelled.
- **No aliases or re-exports**: `ImportRecord::alias` is always empty (`ruby.rs:375`).

## 05. Type inference & receiver typing

Supported:

- **Construction receiver.** `Foo.new(...).m` and `(Foo.new).m` set `typed_receiver`, which looks `m` up on `Foo` and then up its bases, and is never fanned out by name (`receiver.rs:229-239`; CHANGELOG lines 930-940). Fixture `p2`: `Dog.new(3).speak` bound high. Fixture `p3`: `Shop::Derived.new.process` bound high to `Shop::Base::process` through the base chain.
- **`CONSTRUCTING_METHODS = [("ruby", ["new"])]`** (`crates/cgg-core/src/stdlib.rs:58`).
- **Untyped receiver.** A receiver whose name is in `stdlib/ruby.txt` (`each`, `call`, `map`, …) is not fanned out by name. Fixture `p6`: `items.each` stayed unresolved as `candidates-in-other-files` even though `Coll::each` exists.

Not supported:

- **Local variable typing.** `d = Dog.new(2); d.speak` is not typed: the site was refused as `fanout-cap-exceeded` with 6 candidates (fixture `p2`). `ruby.rs` never writes `FileFacts::local_types`.
- **Parameter typing**: not applicable (Ruby has no annotations), and **RBS/Sorbet signatures** are not read.
- **Return-type typing.** `d2 = Dog.make; d2.speak` and `chain_a.chain_b` are not typed by return type (fixture `p2`, `p4`). `build_return_type_map` finds no Ruby return types because `extract_return_type` has no Ruby form (**unverified**, inferred from the fixture result).
- **Instance-variable or field typing** (`@repo = Repo.new; @repo.find`): nothing in `ruby.rs` records `class_fields`.
- **Closure parameter typing** (`closure_types.rs`), **implicit receivers** (`instance_eval`/`instance_exec` blocks, the DSL `self`), **weak receiver types** and **`declared_type`** are not used by Ruby. The `ruby` entry in `type_hints.rs:349` is only a profiling tally name (`lang_tallies_table!`).

## 06. Inheritance, dispatch & overloading

Supported:

- **Base records**: `class X < Y` superclass plus body-level `include M`, `extend M` and `prepend M` constants are stored in every method's `DefRecord::base_types` (`ruby.rs:66-70, 178-185, 316-345`).
- **Bare-call scope (WS8).** A bare call reaches the calling class's members, its superclass's and its included modules' members (`with_ancestors`, `cross_file.rs:4039`), or a top-level def. It never reaches an unrelated class's method (`bare_call_reaches`, `cross_file.rs:4142-4152`; CHANGELOG lines 856-859). Fixture `p1`: `greet` bound to `Base::greet` and `mixed_in` to `Util::mixed_in` (an included module), both high. Fixture `p2`: `bark` in `Runner` did not bind to `Dog::bark`.
- **`member_of_calling_class` runs for Ruby** before step 1 (`cross_file.rs:5648-5666`). That includes overrides below the calling class and the fan-out cap on them. Fixture `p6`: `Coll::inside` (class reopened in another file) bound `puts` to `Coll::puts` at high.
- **Typed receiver up the base chain**: `resolve_via_bases`. Fixture `p3`: `Shop::Derived.new.process` reached `Shop::Base::process`.
- **`inheritance_fanout`**: `"ruby"` is in `INHERITANCE_LANGS` (`crates/cgg-resolve/src/dispatch.rs:110-123`). With `--dynamic-dispatch` (and always under `--dead-code`), fixture `p3` gave `Shop::Base::step -> Shop::Derived::step` (`low`, `dynamic`, `dispatch:inheritance`).
- **Class reopening across files** is treated as one owner path: `unique_owner_names` lists `ruby` as a language whose qn carries the owner path (`cross_file.rs:4383-4384, 4411`), as does `qn_carries_module` (`cross_file.rs:4775`).

Not supported or partial:

- **`super` (zsuper) and `super(...)`** draw no edge (fixture `p3`, section 03).
- **Classes with no methods have no bases.** There are no `ClassDecl` records, so in `class Empty < Base; end`, `Shop::Empty.new.validate` stayed unresolved as `candidates-in-other-files` (fixture `p3`).
- **Intra-file preempts ancestry.** In fixture `p3`, a reopened `class Shop::Base` in `derived.rb` calls `step`. Intra-file found only `Shop::Derived::step` in that file and bound it at **high**. The real target is `Shop::Base::step` in `base.rb`, and `Derived` is a subclass, not an ancestor of `Base`. Intra-file does not apply the ancestry rule that cross-file does. This is a wrong high edge. Also in `p3`, `Shop::Base::process -> step` bound only `Shop::Base::step` at high intra-file. `Derived::step` is reached only by the low `--dynamic-dispatch` edge, not by `overrides_below`.
- **`extend self` / `module_function` singleton semantics** are not modelled. `Tools.mf` still bound because of the name and owner match (fixture `p7`).
- **`include` inside a method or conditional** is deliberately ignored (`ruby.rs:312-315`).
- **Refinements (`refine`/`using`)**: no handling in `ruby.rs`.
- **Interface/trait fan-out** (`dispatch::fanout`): `signals().impls = true` (`ruby.rs:25`), but `ruby.rs` never sets `trait_impl_target`, so nothing reaches it. Dead-code capabilities nonetheless report Ruby `dispatch: full`, because that is taken from the declaration (`crates/cgg-resolve/src/deadcode/caps.rs:150-185`).
- **Overloading**: Ruby has none. Ruby is not in `OVERLOAD_ARITY_LANGS`, and no arity is recorded. Redefinitions bind to every copy at medium (fixture `p8`).
- **Extension methods**: not applicable. Monkey-patching an outside class (`class String; def x`) is just another owner `String`.

## 07. Scoping & shadowing

Supported:

- **Plugin-level local tracking**, which mirrors Ruby's own parser rule. Parameters (including destructured, optional, keyword, splat and block parameters), assignment targets, multiple assignment, `rescue => e` and `for x in` bind locals. `def`, `class` and `module` start a fresh scope, and blocks and lambdas inherit the enclosing one (`ruby.rs:71-142, 186, 222, 225-251`). A bare identifier that is a local is not recorded as a call at all. Test `a_bare_identifier_that_is_no_local_is_a_paren_less_call`. Fixture `p2`: `bark = 1; bark` drew nothing.
- **Lexical nesting check**: a definition nested inside the calling function is always in scope (`bare_call_reaches`, `cross_file.rs:4126-4135`). This applies to every language.
- **Ambiguity in a file** is refused (`AmbiguousInFile`), not guessed. Test `intra_file::ambiguous_name_flags_unresolved` uses `ruby` facts. `bare_call_is_never_a_method` deliberately excludes Ruby, because a bare name is an implicit-self call (`crates/cgg-resolve/src/intra_file.rs:46-53, 315-317`).

Not supported:

- **`FileFacts::locals`**: Ruby has no `locals.rs` spec (`crates/cgg-lang/src/locals.rs:380-391` lists JS/TS, Python, Lua, Scala, Kotlin, Swift, Rust, Dart, OCaml, Zig and others, but not Ruby). So the resolver's `shadowed-by-local` reason and receiver-head-is-local checks never fire for Ruby. Shadowing works only through the plugin omitting the paren-less record.
- **Lexical narrowing and positional shadowing** of nested definitions: Ruby is not in the list (see CLAUDE.md, "Lexical narrowing of nested definitions applies to …").
- **Conditional definitions**: no `DefRecord::conditional` (fixture `p8`, both branches bound).

## 08. Macros, preprocessor & codegen

Not applicable as macros: Ruby has no macro or preprocessor system.

Metaprogramming codegen, the closest analogue:

- `define_method`, `attr_*`, `alias_method`/`alias`, `class_eval`/`instance_eval` strings, `method_missing`-based dispatch and `Struct.new`/`Class.new` blocks are not expanded (fixtures `p1`, `p7`). Nothing is tagged `macro:<name>`.
- `FileFacts::name_mentions` is never written by `ruby.rs`.
- ERB templates are not parsed (section 01).

## 09. FFI & cross-language links

None:

- **`ffi::link_ffi`** has no Ruby coverage. `crates/cgg-resolve/src/ffi.rs` mentions neither `ruby`, `rb_define_method` nor `attach_function`. C extensions (`rb_define_method(klass, "name", c_fn, n)`), the `ffi` gem's `attach_function`, `Fiddle`, and Rust `magnus`/`rutie` bindings are not linked.
- **Descriptor links** (`descriptor.rs`): no Protobuf → Ruby rpc link.
- **Markup code-behind** (`markup.rs`): not applicable.

## 10. Frameworks & entry points

Every rule in `crates/cgg-core/src/frameworks/rules.rs` with `language: "ruby"` (17 rules; no multi-language rule includes Ruby):

| rule id (line) | kind | trigger | node |
|---|---|---|---|
| `rails` (933) | network | detect imports `rails`, `action_controller`, `actionpack`, or paths `config/routes.rb`, `app/controllers/`. Registrars `get post put patch delete root match resources resource namespace scope`, with `string_targets: true` so `'photos#index'` decodes to `PhotosController#index` (`crates/cgg-resolve/src/frameworks/mod.rs:1910-1924`). Base types `ApplicationController`, `ActionController::Base`, `ActionController::API` combined with methods `index show new edit create update destroy`. | yes |
| `sinatra` (972) | network | import `sinatra`; registrars `get post put patch delete options head`. The trailing `do … end` block becomes a `handler_at_<line>` closure (`ruby.rs:416-477`, `registrar.rs:238-247`). | yes |
| `grape` (989) | network | import `grape`; registrars `get post put patch delete route` | yes |
| `sidekiq` (1006) | queue | import `sidekiq` or paths `app/workers/`, `app/jobs/`. Base types `Sidekiq::Job`, `Sidekiq::Worker`, `ApplicationJob`, `ActiveJob::Base` (an `include` counts as a base), method `perform`. | yes |
| `hanami` (2785) | network | import `hanami`; no matchers (gap text only) | yes, but enumerates nothing |
| `roda` (2802) | network | import `roda`; no matchers (gap) | yes, but enumerates nothing |
| `padrino` (2819) | network | imports `padrino*`, path `config/apps.rb`; no matchers (gap) | yes, but enumerates nothing |
| `rake` (2844) | cli | import `rake`; no matchers (gap) | yes, but enumerates nothing |
| `aws-lambda` (4659) | network | imports `aws_lambda_ric`, `aws-xray-sdk`, `aws-sdk-*` (9 listed); method `lambda_handler` | yes |
| `gcp-functions` (4694) | network | import `functions_framework`; registrars `http`, `cloud_event`, `typed` (inline block handler) | yes |
| `minitest` (6979) | test | see section 11 | yes |
| `rspec` (7009) | test | gap only | — |
| `rails-callbacks` (10724) | lifecycle | imports `rails`, `active_record`, `activesupport`, `active_job`, …; 37 registrars (`before_save`, `after_commit`, `before_action`, `around_perform`, `rescue_from`, …), `string_targets: true`, so the symbol arguments name the handler | no (root mark only) |
| `actioncable` (10789) | network | imports `rails`, `action_cable`, `actioncable`, `actionpack`; base types `ApplicationCable::Channel`, `ActionCable::Channel::Base`, `ApplicationCable::Connection`, `ActionCable::Connection::Base`; no methods, so every member matches | yes |
| `rack` (10811) | network | import `rack`; no matchers (gap: `call(env)` is too common, and `config.ru` is not parsed) | no |
| `puma` (10828) | lifecycle | import `puma`, path `config/puma.rb`; no matchers (gap) | no |
| `sidekiq-lifecycle` (10846) | lifecycle | import `sidekiq`; no matchers (gap) | no |

Confirmed in fixtures:

- **`rails`** (fixture `rails`): the `get`, `post` and `root` string targets were minted as entry nodes to `PhotosController::index` and `::show`, and `UsersController::index` got a base-type entry. Test `crates/cgg/tests/frameworks.rs::shape_e_rails_string_routing_reaches_the_controller_action`. Unit test `rails_string_routing_decodes_to_a_controller_action`.
- **`rails-callbacks`**: 4 root marks (`load_photo`, `normalize`, `notify`, `create`).
- **`sidekiq`**: `HardWorker::perform` matched through `include Sidekiq::Job` with no `require`, via the `app/workers/` path.
- **`sinatra`, `grape`, `aws-lambda` and `minitest`** all fired in fixture `fw`.
- **`gcp-functions`**: `crates/cgg/tests/frameworks.rs` CLOUD_CASES `gcp-functions/ruby`, with expected `gcp-functions::http('hello')`. **`aws-lambda`**: CLOUD_CASES `aws-lambda/ruby`.

Helper tables: `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for`, `self_module_markers_for` and `detect_calls_for` have no Ruby entries (`rules.rs:11627-11900`, grep shows only `TEST_CLASS_MEMBERS_ARE_ROOTS`).

Not supported or partial:

- **`resources :users`** is not expanded to the seven RESTful actions. The symbol produces only a value-ref record (fixture `rails`, `users` value-ref-no-enclosing). `UsersController::create` was rooted only by base type.
- **`on: :create` false root mark.** The `rails-callbacks` `string_targets` option captured `on: :create` inside `after_commit :notify, on: :create` as a handler. With that file present, `UsersController::create` became a `rails-callbacks` root mark and lost its `rails` entry node (fixture `rails` vs `rails2`). The mechanism is inferred from the root-mark count, and the exact code path is **unverified**.
- **Detection is per project, not per file.** In fixture `fw`, `sinatra` and `grape` each claimed the other's handlers (`sinatra::API::handler_at_4`, `grape::get('/hi')`), so every handler got two entry nodes.
- **`actioncable`** did not fire when nothing required rails or actioncable, because it has no `detect_paths` (fixture `rails`: `ChatChannel::subscribed` and `::speak` were reported dead).
- **Grape `get :status do`**: the route is a symbol, so the entry is named after the qn (`API::handler_at_4`), not the route.
- **Rails `controllers` DSL, Rails engines and `draw(:file)` route splitting, Hanami, Roda, Padrino, Rake tasks, Puma hooks, Rack `call(env)` and the Sidekiq server lifecycle** are listed only as gaps (see the `gap` strings).

## 11. Test harness entries

Supported:

- **`minitest`** (kind `test`, `rules.rs:6979`) is triggered by base type, not annotation. The bases are `Minitest::Test`, `Minitest::Spec`, `Test::Unit::TestCase`, `TestCase`, `ActiveSupport::TestCase` and `ActionDispatch::IntegrationTest`, with method patterns `test_*`, `setup`, `teardown`, `before_setup` and `after_teardown`. It mints one node, `<framework-entry>::test::minitest`, with entry edges to each case (fixture `fw`).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** includes `ruby` (`rules.rs:11806`). The other members of a test class become test root marks without a node (`frameworks/mod.rs:898-906`). Fixture `fw`: `ThingTest::support_helper` and `::not_a_test` were rooted as `test:minitest:base-type`. Fixture `p7`: `AsTest::used_in_dsl` was rooted under `ActiveSupport::TestCase`.
- **Built-in name rule** `is_test_name("ruby", n) = n.starts_with("test_")` (`crates/cgg-resolve/src/deadcode/roots.rs:985`) applies in any file. Fixture `p7`: `Gen::test_named_outside` in non-test `misc.rb` was a `TestEntry` root.
- **Test-file classification**: `*_spec.rb` and `*_test.rb` (`crates/cgg-core/src/testfile.rs:105`). Findings in test scope are hidden unless `--include-tests` is passed (fixture `fw`: `spec_util` was not reported).
- **`--no-test-entries`** applies generically.

Not supported:

- **RSpec**: `it`/`describe`/`let`/`before` blocks are not enumerated. Only the `gap` text is reported (`rules.rs:7009-7022`, fixture `fw`: "seen, no rules").
- **ActiveSupport `test "name" do … end` DSL** blocks are anonymous. Their calls are attributed to no callable, and the methods they call are kept live only by the class-member root mark (fixture `p7`).
- **Cucumber step definitions, Minitest::Spec `it` blocks and benchmark-ips**: no rules.

## 12. Dead-code analysis

Supported:

- **Root kinds that fire for Ruby** (fixture `fw` capabilities: `root_rules_active`): `builtin:main` (a def named `main`), `builtin:lifecycle`, `builtin:test-name`, `framework:entry-node`, `framework:rule` and `framework:test-harness`.
- **`builtin:lifecycle`** covers `CONVENTIONAL_METHODS` (`initialize`, `to_s`, …) and dunder names, because Ruby is in `DUNDER_LANGS` (`roots.rs:160-170, 868-886`).
- **Top-level invocation rule** (`NoEnclosingCallable` / `ValueRefNoEnclosing`, `roots.rs:552-575`). Script-level `b()` and `c 1` rooted `b` and `c` (fixture `tl`), and class-body `before_action :x` symbols root `x`.
- **Signals**: `value_references: full` and `dispatch: full` are reported from the declared signals (section 06 notes that dispatch is declared but never produced). `visibility`, `attributes`, `exports` and `test_tagging` are `none`. The language class is `degraded`, so `max_confidence` is `medium` (`caps.rs:85-95, 160-172`; fixture `fw` capabilities JSON). Every Ruby finding carries the `language-lacks-attributes` and `language-lacks-visibility` risks (fixture `rails`).
- **Reflection screen**: `send`/`public_send`/`method`/`respond_to?`/`instance_variable_get` literals cap same-named findings at medium (`crates/cgg-resolve/src/deadcode/mod.rs:626-634`). For Ruby this changes nothing, because medium is already the ceiling.
- **`inheritance_fanout`** runs under `--dead-code`, so overrides of in-tree base methods are reachable (`dispatch.rs:110-123, 270`).

Not supported or partial:

- **A top-level paren-less call does not root its target.** Fixture `tl`: `a` (written bare at script level) was reported as an unreferenced method, while `b()` and `c 1` were rooted. The paren-less site leaves no audit record (`crates/cgg/src/lib.rs:1070-1086`), so the top-level-invocation rule never sees it. The same thing caused `rake_helper` called inside a `task` block to be reported dead (fixture `fw`).
- **No `implements-external-contract` cap.** `external_contract` applies only to Rust, partial methods, override markers or Python (`deadcode/mod.rs:470-531`), so a Ruby method on a class with an outside base (`< ActiveRecord::Base`) gets no doubt evidence from that.
- **No visibility.** `private` helpers and public API are indistinguishable. There are no exported or public roots for gems: `exports: none`, and nothing roots a gem's public methods.
- **Not rooted**: `method_missing`, `respond_to_missing?`, `inherited`, `included`, `self.extended`, `<=>`, `each` (Enumerable contract), `to_str` and `coerce`. They are not in `LIFECYCLE` or `CONVENTIONAL_METHODS` (`roots.rs:96-170`), so they rely on there being a call. This follows from the code and was not run as a fixture (**unverified** in output).

## 13. Builtins & stdlib tables

Supported:

- **`crates/cgg-core/src/stdlib/ruby.txt`**: 792 lines, including operators (`!`, `==`, `[]`, `<=>`, …), Kernel and Object methods, Enumerable methods and 53 capitalised core and stdlib constants (`Array`, `JSON`, `File`, `Kernel`, `Logger`, …). It is loaded at `crates/cgg-core/src/stdlib.rs:195, 236`.
- **Effect.** Bare and untyped calls to these names are classified as stdlib, not fanned out by name (fixture `p6`: `items.each` was not bound to `Coll::each`). With `--include-stdlib` they become `<stdlib>::name` exit nodes (fixture `p1`: `<stdlib>::puts`, `<stdlib>::JSON::parse`, `<stdlib>::send`). A workspace member of the same name still binds through the calling class (fixture `p6`: `puts` inside `Coll` bound `Coll::puts`).
- **`CONSTRUCTING_METHODS`**: `("ruby", ["new"])` (`stdlib.rs:58`).

Not supported:

- **No `CAST_INTRINSICS` entry**, which is correct because Ruby has no casts. No implicit-package table.
- **No `rust_types`-style wrapper tables.**
- **Stdlib names hide workspace methods with no class context.** For example, `f.call` to a workspace `call` stays unresolved (fixture `p6`).

## 14. Confidence & certainty

Paths that produced `high` in fixtures:

- intra-file unique candidate: `Service::run -> Service::helper2`, `Dog::speak -> Dog::bark`.
- cross-file unique name through a require: `top_helper`.
- `Util.fmt` / `Util::fmt` (unique owner).
- ancestor or included-module members: `Base::greet`, `Util::mixed_in`.
- typed construction receivers: `Dog.new(3).speak`, and `Shop::Derived.new.process` through its base.

Ruby counts as a language whose qns carry their owner path (`qn_carries_module`, `cross_file.rs:4775`), which allows high results on exact owner paths.

What gives `medium`:

- by-name match on an untyped receiver: `unknown.purr -> Cat::purr`, `c&.safe_target`, `chain_a.chain_b`
- fan-out over copies or redefinitions (fixtures `p5` `c3`, `p8`)
- a class method reached through two same-named classes in different files (`K.s`, fixture `p5`)

Everything over `--fanout-cap` (5) is refused (fixture `p2`: 6 `speak` candidates).

Wrong `high` edges observed:

- the reopened class binding to a subclass's method intra-file (fixture `p3`, section 06)
- a `require_relative` call binding to the non-required copy (fixture `p5`, section 04)

Mermaid marks arrows with no high site with `?` (fixture `p2`: `N10 -->|?2x| N5`). CHANGELOG lines 1418 and 1462 record corpus figures of Ruby high 0 / 4,482 and medium 486 / 3,150, then medium 0 / 564 removed in fix passes.

## 15. Dynamic use & reflection

Supported:

- **Reflection capture** (`crates/cgg-lang/src/plugins/dynuse.rs:31-39`): the callees `send`, `public_send`, `method`, `respond_to?` and `instance_variable_get`, with string, symbol or simple-symbol literal arguments that are identifier-shaped. This runs only under `--dead-code` (`ExtractCtx::deadcode_signals`, `ruby.rs:53-55`). Test `dynuse::ruby_send_names_a_method`. Captured names are a suppression-only signal (screen `reflection`) and never edges (CHANGELOG lines 736-741).

Not supported:

- **Module-load screen**: `DYN_MODULE_LOADERS = ["require", "importScripts", "import_module"]` (`crates/cgg-core/src/facts.rs:165`), but `require` is not in Ruby's REFLECTIVE list, so no Ruby `module-load` dyn-use is produced.
- **Not captured**: `__send__`, `instance_variable_set`, `const_get`, `define_method`, `instance_method`, `public_method`, `try` (Rails), `Object.const_get("X")`, `constantize`, `eval`, `instance_eval`, `class_eval` and `method_missing` dispatch.
- **No `BLIND_SPOTS` entry for Ruby** in `caps.rs:43-67`, so the report names no Ruby-specific blind spot.

## 16. Known gaps & limitations

Bugs and gaps found while cataloguing (fixtures in the scratch directory):

1. **`Foo.new` never reaches `initialize`** (`p1`, `p6`). The README row "initialize → Constructor" (`README.md:497`) describes only the node kind.
2. **`super` / `super()` draw no edge** (`p3`).
3. **No local-variable typing** (`x = Foo.new; x.m` is refused or fanned out) and no return-type typing (`p2`).
4. **`require` does not anchor to the required file.** A wrong-file high edge was observed (`p5`).
5. **Intra-file binding ignores ancestry** in a reopened base class. A wrong high edge to a subclass method was observed (`p3`).
6. **Doubled qn prefix**: `module Shop; class Shop::Nested` gives `Shop::Shop::Nested::n` (`p3`).
7. **A class with no methods records no bases**, because there are no `ClassDecl` records (`p3`).
8. **Setter (`x.y = 1`) and operator (`a + b`, `a[i]`) calls** don't reach their defs (`p4`).
9. **`define_method`, `attr_*` and `alias_method`** produce no callables. `Struct.new`/`Class.new` bodies are owner-less (`p1`, `p7`).
10. **No visibility**, so Ruby is always `degraded` with medium-capped dead-code (section 12).
11. **A top-level paren-less call does not root its target** (`tl`).
12. **Redefinitions and conditional defs bind to every copy** (`p8`).
13. **`&method(:x)` / `&:sym` / `method(:x)` are not value references** (`p4`).
14. **Framework issues**: detection is per project, so Sinatra and Grape cross-claim each other's handlers (`fw`); `on: :create` mis-roots `create` (`rails`); `resources` is not expanded; ActionCable is missed without a require; RSpec, Rake, Hanami, Roda, Padrino, Puma, Rack and the Sidekiq lifecycle have gap text only.
15. **Not detected**: `Gemfile`, `Rakefile`, `config.ru`, `.erb` and `.rbw` files (`det`).

Documented elsewhere:

- **CHANGELOG** line 1350 ("Not covered (WS14): … Ruby `require`").
- **Rule `gap` strings** for `hanami`, `roda`, `padrino`, `rake`, `rspec`, `rack`, `puma` and `sidekiq-lifecycle` (`rules.rs` lines listed in section 10).
- **No `#[ignore]` tests** for Ruby: `grep -rn "ignore" crates/cgg-lang/src/plugins/ruby.rs` finds none.
- **Thin test coverage.** The plugin has 4 unit tests. Elsewhere there are 1 receiver test, 1 dynuse test, 1 intra-file test, 1 `resolve.rs` integration test and 3 framework integration tests, and there are no `.rb` fixture files under `crates/`.

## Summary scorecard

| section | support | one-line note |
|---|---|---|
| 01 Detection & parsing | partial | `.rb`/`.rake`/`.gemspec` plus `ruby`/`irb` shebangs; Gemfile, Rakefile, config.ru and ERB are skipped |
| 02 Callables extracted | partial | defs, singleton defs, initialize and operators work; no attr_*/define_method/alias/visibility; doubled qn for `Shop::X` inside `module Shop` |
| 03 Call-site forms | partial | calls with and without parens and `Foo.new.m` are typed; `super`, setters, operators, `.()`, `&method` and `new→initialize` are missing |
| 04 Imports & module resolution | minimal | literal `require`/`require_relative` only; does not anchor to the file (wrong-file high edge); no heads/modules/packages |
| 05 Type inference | minimal | only `Foo.new` construction receivers; no local, ivar or return typing |
| 06 Inheritance & dispatch | partial | superclass and include bases, WS8 bare-call scope, inheritance fan-out; no super, no ClassDecl, intra-file ignores ancestry |
| 07 Scoping & shadowing | partial | plugin-side local tracking per Ruby's rule; no `FileFacts::locals` or lexical narrowing |
| 08 Macros & codegen | none | metaprogramming (define_method, attr_*, Struct.new) not modelled |
| 09 FFI & cross-language | none | no C-ext/ffi-gem/magnus links |
| 10 Frameworks | partial | rails, sinatra, grape, sidekiq, rails-callbacks, actioncable, aws-lambda and gcp enumerate; 8 rules are gap-only; per-project detection cross-claims |
| 11 Test harness | partial | minitest by base type plus `test_` names and class-member roots; RSpec and the `test "x" do` DSL are not enumerated |
| 12 Dead-code | minimal | degraded class (medium max), no visibility or exports; top-level paren-less calls don't root |
| 13 Builtins & stdlib | full | 792-name `ruby.txt` plus `CONSTRUCTING_METHODS` `new` |
| 14 Confidence | partial | high paths exist, but two wrong-high patterns were observed (require copies, reopened class) |
| 15 Dynamic use | partial | send/public_send/method/respond_to?/ivar_get captured; no const_get/eval/module-load |
| 16 Known gaps | n/a | 15 itemised gaps above |
