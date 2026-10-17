# cpp

The C++ plugin is `CppPlugin` (`crates/cgg-lang/src/plugins/cpp.rs`, 2,746 lines including inline tests). It parses C++ with the crates.io grammar `tree-sitter-cpp` 0.23 (`Cargo.toml:89`, `tree_sitter_cpp::LANGUAGE`, cpp.rs:45-47). Nothing is vendored.

A single recursive `CppWalker` (cpp.rs:101-858) keeps two stacks:

- a namespace/class `scope` stack;
- a `class_stack` with a parallel `bases` stack.

The walker records:

- function definitions and prototypes, including out-of-line `Class::method` bodies;
- field and local types;
- type aliases, namespace aliases and object-like macros;
- `#include` directives;
- call sites, `new` expressions and member-pointer tables.

It also runs the shared registrar-argument capture for registration calls.

The plugin declares the signals `attributes`, `unreachable` and `impls` (cpp.rs:36-43). It does **not** declare `visibility`, `exports`, `test_defs`, `value_refs` or `dyn_uses`. There is no `resolver_kind` override, so resolution uses the generic pipeline.

C++ shares several passes with C and Objective-C (the "C family", `cgg_core::lang_family`, lang_family.rs:28):

- **Macro expansion:** `c_macro.rs`, run for C and C++ only (cgg/src/lib.rs:526-534).
- **Prototype/body unification:** `cpp::unify_declarations`, which runs over C, C++ and Obj-C (cgg/src/lib.rs:535).
- **Include-closure resolution:** in `cross_file.rs`.

C++-specific resolver branches:

- `cross_file.rs`: `cpp_virtual_dispatch` at 2920, the untyped-receiver fallback at 2524, the `several` typed-call rule at 6153, and the include-declared external escape at 5347.
- `intra_file.rs`: class-first bare lookup at 449, and the scope-qualified rule at 524.
- `heads.rs`: `cpp_scopes` at 168/194, namespace aliases at 625, and the external-namespace refusal at 694.
- `type_hints.rs`: the type-alias index at 121, member chains at 579, macros and aliases at 640, and the untyped fallback at 933.

Fixtures cited below live under `<scratch>/parity/cpp/` (f1 … f14). Each was run with `target/release/cgg` 0.11.0 using `-t json -o FILE` and dumped with `show.py` in the same directory.

## 01. Detection & parsing

**Supported:**

- **Extensions:** `.cc .cpp .cxx .C .hpp .hh .hxx` (cpp.rs:33-35). The detect.rs test table maps `j.cpp` and `k.cc` to cpp (detect.rs:688-689). In fixture f11, `x.cc` was detected as `extension:.cc`.
- **`.h` disambiguation** (`header_verdict`, detect.rs:226-261). `.h` belongs to the C plugin (c.rs:25-27), but a `.h` is re-assigned to cpp in two cases:
  - **Content sniff** (`has_cpp_content`, detect.rs:270-327), over the first 8 KiB with comments and strings stripped. The triggers are:
    - a raw string `R"(`;
    - a C++ standard-library include (`<vector>`, or any `<…++…>`; the list is `CPP_STDLIB_HEADERS`);
    - `namespace`;
    - `template <`;
    - `enum class`;
    - a base clause;
    - an access specifier;
    - `using`;
    - `constexpr` or `noexcept`;
    - any `::`;
    - a `virtual` member.

    `extern "C"` is deliberately not a trigger. In fixture f11, `cls.h` (`class Foo { public: virtual void x(); };`) was detected as `header-content:cpp`.
  - **Sibling heuristic:** a same-stem `.cpp/.cc/.cxx/.hpp/.hh/.hxx/.C` file in the same directory (detect.rs:236-254). In fixture f11, `x.h` next to `x.cc` was detected as `header-heuristic:cpp`, and `plain.h` was detected as `header-heuristic:c`.
  - Tests: `header_disambiguation_prefers_cpp_when_sibling_exists` (cgg/tests/detect.rs:88), and the detect.rs unit tests at 742-1060.
- **Shebangs:** none, because `shebangs()` is not overridden.
- **Re-parse/fallback:** none. Unlike C#, there is no `#if` blanking. A file with syntax errors is still extracted (`parse_status: error`). The plugin also cleans up error recovery locally:
  - `clean_qualified` (cpp.rs:463-504) drops a return type or macro that the grammar folded into a qualified declarator name. Test: `a_recovered_return_type_is_not_a_scope_of_the_name` (cpp.rs:1820).
  - `try_record_prototype` refuses a "declaration" whose text contains `{` before the declarator. This is the macro-prefixed method body case (cpp.rs:590-615). Test: `a_macro_prefixed_method_body_mints_no_phantom_prototype` (cpp.rs:1910).
  - A block-scope `declaration` is never a prototype. This is the most-vexing-parse case `std::ifstream info(path);` (cpp.rs:605). Test: `a_local_constructed_with_arguments_is_not_a_prototype` (cpp.rs:1804).
- **Depth skip:** shared by all languages. A tree deeper than `MAX_TREE_DEPTH` (4,000) is skipped as `too-deep` (cgg/src/lib.rs, `cgg_lang::exceeds_depth`).
- **Minified/long-line handling:** not applicable. `max_line_bytes` is not overridden, and there is no `locals.rs` spec whose 2,000-byte rule could apply.
- **`vendor/` and other deny-list directories** are skipped by cgg-walk, as for every language.

**Not supported:**

- **CUDA files are not detected.** `.cu` and `.cuh` are not detected. In fixture f8, `cu/k.cu` was skipped as `unknown-extension`. This holds even though the `cuda` framework rule exists, and a frameworks unit test builds facts for `k.cu` (frameworks/mod.rs:3096). In practice CUDA kernels are only seen in `.cpp`/`.cc`/`.h` files.
- **Template/inline includes and other C++ extensions are not detected:** `.inl`, `.tpp`, `.ipp`, `.ino` (Arduino) and `.c++`. In fixture f11 all four test files were skipped as `unknown-extension`. A grep of crates/ for these extensions finds none.
- **`.mm` (Objective-C++)** belongs to the objc plugin (objc.rs:15-17), not cpp.
- **No preprocessor evaluation.** Every `#if`/`#ifdef` branch is parsed:
  - Fixture f11: the `#if 0 … void disabled() {} … #endif` body is extracted as a callable.
  - Fixture f7c: both `impl()` copies under `#ifdef/#else` are extracted, and a call to `impl()` binds both at medium.

## 02. Callables extracted

**How names are formed.** A qualified name (QN) is the open `scope` stack (namespaces, then classes) joined by `::`, plus the name (`qn`/`qualified_name`, cpp.rs:127-133, 361-377):

- An out-of-line declarator that already spells the scope (`math::Calc::add` inside `namespace math`) is not prefixed twice (cpp.rs:371-376). Test: `out_of_line_inside_namespace_matches_the_class` (cpp.rs:1967).
- An anonymous namespace becomes the segment `(anonymous)` (cpp.rs:147-153). Test: `anonymous_namespace_is_its_own_scope` (cpp.rs:2307).
- An unnamed class or struct pushes no scope (cpp.rs:170-174).

**Variants.** A definition's variant (cpp.rs:379-457) is one of:

- `Constructor`: the name equals the enclosing class, or the immediate qualifier.
- `Destructor`: `~X`.
- `InherentMethod`: a `field_identifier`, a qualified name, or an operator.
- `FreeFunction`: an `identifier` declared at global scope.
- **Important:** an `identifier` declared inside a namespace becomes `InherentMethod`, because the scope is not empty (cpp.rs:393-399). Namespace-level free functions are therefore emitted with kind `method`. Fixture f1: `app::detail::helper`, `app::free_fn` and `app::(anonymous)::anon_fn` all have kind `method`, while global `main` has kind `function`.

**Attributes.** Each definition carries:

- the CUDA qualifiers `__global__`/`__device__`/`__host__`, matched on the declaration head only (`cuda_qualifiers`, cpp.rs:2713-2729; test `kernel_qualifier_is_captured_from_the_declaration_head`);
- `virtual`, when the definition has a `virtual` token or a pure-virtual clause (`is_virtual_method`, cpp.rs:937-944).

Its `base_types` are the enclosing class's bases (cpp.rs:336-351). `visibility` is always empty (cpp.rs:346).

**Supported shapes:**

- **Global free function.** `int main() {}` is a `function`. Fixture f1.
- **Namespace function.** `namespace app { int free_fn(){} }` becomes `app::free_fn`, with kind `method` as noted above. Fixture f1. Test: `namespace_qualified_names` (cpp.rs:1842).
- **In-class method.** `void draw() const {}` becomes `app::Widget::draw`. Fixture f1.
- **Constructor and destructor**, in-class or out-of-line, including a pointer return such as `int *Robot::foo()`. Tests: `class_method_and_constructor` (cpp.rs:1858) and `out_of_line_constructor_destructor_and_pointer_return` (cpp.rs:2004). Fixture f1 extracted `Widget::Widget` ×2 and `Widget::~Widget`.
- **`= default` and `= delete`** count as definitions: `has_body` is set (cpp.rs:318-325). Fixture f13: `Vec(const Vec&) = delete;` becomes a `constructor` node.
- **Pure virtual** `virtual void paint() = 0;` becomes a node tagged `virtual` with `has_body=false`, so a later body can absorb it. Fixture f1: `Widget::paint` has `attrs=['virtual']`.
- **Out-of-line member.** For `void Engine::start(int n) {}`, the simple name is the last segment and the QN is the declarator text (`qualified_fn_info`, cpp.rs:427-457). Test: `out_of_line_simple_name_is_the_method` (cpp.rs:1953). Fixture f3.
- **Nested class member**, in-class (`Widget::Inner::run`, fixture f1) or out-of-line (`void Vec::Inner::f()`, fixture f13).
- **Static member function.** `static Widget make()` is an `InherentMethod`; there is no `StaticMethod` variant for C++. Fixture f1.
- **Operator overloads, in-class or out-of-line:** `operator()`, `operator+=` and `Vec Vec::operator+(…) const` become `Widget::operator()` and similar (`operator_name`, cpp.rs:413-415). Fixtures f1 and f13.
- **Function templates and member templates.** The template parameters are not part of the name: `tmax` and `Widget::conv` (`template_declaration` is walked through, cpp.rs:183-187). Fixture f1.
- **Out-of-line members of class templates.** `template<class T> void Box<T>::put(T)` gets the QN `Box<T>::put`, keeping the template arguments. `unify_declarations` strips them to match the in-class prototype (`strip_template_args`, cpp.rs:1467-1481). Test: `cpp_template_member_prototype_unifies_with_its_out_of_line_body` (cgg/tests/resolve.rs:884). Fixture f13: `Box<T>::put`, `Box<int>::get`.
- **Prototypes.** A `declaration` or `field_declaration` with a function declarator becomes a `has_body=false` node (cpp.rs:590-625). Fixture f10: `extern "C" int asm_func(int);` stays a node, because no body exists.
- **Prototype/body unification** (`unify_declarations`, cpp.rs:1335-1464) runs across C, C++ and Obj-C files. The key is the template-stripped QN plus an `overload_key`: the normalized parameter list and cv/ref qualifiers, with names, defaults, comments, `std::` and array/pointer differences removed (cpp.rs:1484-1770).
  - A prototype is dropped when a body with that key exists.
  - `virtual` and the bases are copied onto the body.
  - The header keeps its claim to the name for `#include` reachability (`unified_decls`).
  - Tests: `prototype_is_dropped_when_the_body_is_in_another_file` (cpp.rs:2318), `enum_comments_and_array_parameters_still_match` (2351), `cv_qualified_unnamed_type_matches_the_named_body` (2373), `std_qualification_and_array_parameters_match_the_body` (2407), `overloads_and_anonymous_bodies_are_not_merged_away` (2429), `c_header_prototype_is_unified_with_cpp_body` (2566) and `unify_copies_virtual_and_bases_onto_the_body` (2286). Fixture f3: `Engine::start`/`stop`/`util` exist only as bodies.
- **Overloads.** Each overload is its own node; nothing merges them. Fixture f6: `Log::write` ×3 and `f` ×2.
- **Function-like `#define`** becomes a `FreeFunction` node tagged `macro` (cpp.rs:627-656). Fixture f7b: `DEF_GETTER` has `attrs=['macro']`.
- **Macro-generated definitions** are tagged `macro:<NAME>` (see §08). Fixture f7b: `get_a`, `Panel::get_b`, `Panel::on_click` and `ns::on_key`.
- **Synthetic framework entries** are listed in §10.

**Not supported or partial:**

- **Lambdas are never callables.** `cpp.rs` has no `lambda_expression` arm (cpp.rs:135-296). Calls inside a lambda are attributed to the enclosing function: in fixture f2, `[p]() { p->area(); }` produced the edge `calls -> Shape::area`. A call through a lambda-holding variable (`lam()`) is `external` with `no-candidate-in-file` (fixtures f1 and f2).
- **Conversion operators are not extracted.** `operator bool() const {…}` and `explicit operator bool()` are both missing from fixtures f1 and f13. The `operator_cast` arm (cpp.rs:413) is never reached. The probable, unverified, cause is that the grammar does not nest `operator_cast` in a `function_declarator`, so `unwrap_function_declarator` (cpp.rs:898-930) returns `None`.
- **Explicit full specialization of a free function template** (`template <> int tmax<int>(int, int)`) is not extracted, because the declarator is a `template_function`, which `fn_info_from_declarator` does not handle. Fixture f1 has only one `tmax` node. A class-member specialization written with a qualified name does work (fixture f13: `Box<int>::get`).
- **Inline `friend` functions get the class as owner.** `friend void swap(Widget&, Widget&) {}` becomes `app::Widget::swap`, a method, although it is a namespace-level function (fixture f1). A friend declaration with an out-of-class body is never unified:
  - Fixture f13: `friend bool operator==(…)` gives a prototype node `Vec::operator==`.
  - The body is the separate node `operator==`.
- **Variant gaps.** There is no `StaticMethod`, `Property` or `abstract` tagging. Pure virtuals are tagged `virtual`, not `abstract`, so the abstract-candidate narrowing of duck fan-out does not apply (grep for `abstract` in cpp.rs finds nothing).
- **Function-pointer fields and variables** are not callables, by design. Test: `function_pointer_field_is_not_a_callable` (cpp.rs:2038).
- **Inheriting constructors** (`using Vec::Vec;`) produce no record (fixture f13).
- **Doctest/Catch2 tests are absent.** A `TEST_CASE("…")` body is not a callable (rules.rs:7915, gap prose; fixture f8). gtest `TEST(Suite, Name)` parses as a function named `TEST`/`TEST_F` (fixture f8 nodes `TEST`, `TEST_F`), so every gtest test shares one QN per macro.
- **Qt `slots:` sections are lost.** A class with `Q_OBJECT` and `public slots:` lost its slot method `onClick` to error recovery; only `Win::wire` was extracted (fixture f8, `src/qt.cpp`).

## 03. Call-site forms

`record_call` (cpp.rs:740-829) emits one `RefRecord` per `call_expression`. Every call records `arity`, the argument count (cpp.rs:820-823). Test: `calls_record_their_argument_count` (cpp.rs:2691).

**Supported:**

- **Bare call.** `bare();` has the name `bare` and an empty receiver. Fixture f2, L25: `calls -> geo::bare`, high.
- **Scope-qualified call.** For `geo::bare()`, `ns::A<int>::f<float>()` and `Type::f()`, `split_qualified` splits the call by tree nodes into a scope receiver and a bare name, and sets `path_receiver=true` (cpp.rs:794-797, 813, 970-993). Tests: `qualified_call_expression` (cpp.rs:1902), `qualified_calls_are_path_receivers_and_aliases_are_recorded` (2601) and `qualified_calls_without_template_arguments_are_unchanged` (2676). Fixture f2, L26: high.
- **Explicit template arguments.** `gen<int>(1)`, `geo::gen<float>()`, `v.tm<int>()` and `v.template tm<int>()` are recorded under the bare name (`callee_name`, cpp.rs:950-968). Test: `template_arguments_are_stripped_from_callee_names` (cpp.rs:2642). Fixture f2, L28/L29/L39/L40: all high.
- **Member calls** `p->area()`, `r.area()` and `v.area()` (`field_expression`). `->` sets `RefRecord::arrow` (cpp.rs:751-754). Tests: `field_expression_call` (cpp.rs:1940) and `an_arrow_call_is_marked` (2144). Fixture f2, L30-32: high.
- **Receiver shapes peeled by `crate::receiver::peel("cpp", …)`** (cpp.rs:767-783):
  - Parentheses, and `(*p).m()`, which sets `deref`, so the call reads as `->`.
  - Casts: `static_cast/dynamic_cast/reinterpret_cast/const_cast` (`CAST_INTRINSICS`, stdlib.rs:35-44), and C casts `((Pump*)x)->m()`.
  - Construction by call: `Foo().m()`, because cpp is in `constructs_by_call` (receiver.rs:46-51).
  - Brace construction: `Foo{}.m()`.

  Each of these sets `typed_receiver`. Fixture f2, L33/L36/L37/L38: high. Fixture f4, L27-28: high.
- **Explicit base-qualified member call.** `this->Base::Close()` has the name `Close` and the receiver `Base` (cpp.rs:787-793). Fixture f5: `this->Mid::draw()` gives `Leaf::draw -> Mid::draw`, high.
- **Chained calls** `p->self()->area()` and `p->get().spin()`: the inner call resolves. The outer call is either fanned out by name at medium (fixture f2, L34; fixture f4, L24) or left unresolved (fixture f2, L35: `Shape::build().area()` is unresolved, `candidates-in-other-files`). Method return types are not used to type a chained receiver (see §05).
- **`new T(args)`** becomes a reference to `T` with arity (`record_new`, cpp.rs:720-738). It binds the constructor, arity-narrowed. Fixture f2, L41: `new Shape(2)` reaches `Shape::Shape(int)`, high.
- **Construction by call.** `Widget(1)` binds the constructors by arity (fixture f1, L12: both one-argument constructors at medium). Fixture f2, L36: `Shape()` binds the zero-argument constructor, high.
- **Indexed member-pointer call** `(m->*table[i])(…)` is recorded as the name `->*` with the table as receiver (cpp.rs:742-750, 1027-1064). `&Type::method` inside a table initializer is recorded as a `MemberPtrTake` (cpp.rs:831-857). These are consumed only by `cpp_virtual_dispatch` (§06). Tests: `member_pointer_table_and_indexed_call_are_recorded` (cpp.rs:2258) and `cpp_member_pointer_table_reaches_overrides` (cgg/tests/resolve.rs:1285).
- **Registrar arguments.** The callee text of every call is passed to `registrar::capture` (cpp.rs:285-290), which is gated on the call's last segment being a registrar verb of some rule (registrar.rs:424-440). Identifier arguments then become value references. For C++ the verbs come from the `aws-lambda` (`run_handler`) and `cpp-callbacks` rules (§10). With `--reference-edges` these show as `?ref` edges (fixture f8, `src/app.cpp`: six `-->|?ref|` arrows).
- **Calls inside macro arguments** are ordinary `call_expression`s and are recorded (fixture f7: `SQUARE(2)` binds the `SQUARE` macro node).
- **Calls at namespace/global scope** in initializers (`int g_init = compute();`) are recorded with no enclosing callable. They feed the `toplevel:invocation` dead-code root (fixture f13).

**Not supported:**

- **Global-qualified `::geo::bare()` is unresolved.** The receiver is `::geo`. Fixture f2, L27: `candidates-in-other-files`, and also in the external bucket.
- **Operator expressions are not call sites.** `v + r` and `a == b` produce no reference, so `operator+` is never called (fixture f2, L47).
- **Call operator on an object.** `v(5)`, where `v` has `operator()`, is recorded as a bare call to `v` and goes to `external` (fixture f2, L48). C++ has no `call_operator_names` step.
- **Function values outside registrar verbs** produce no record. This covers `run_all(callee_value)` and `run_all(&callee_value)` (fixture f2, L49-50: only the `run_all` edge), `std::thread t(fn)`, `std::function<void()> fn = callee_value;` and `fp = &only_by_ptr` (fixture f9: `only_by_ptr` is reported dead). The plugin does not call `registrar::capture_value_refs`, and its signals declare no `value_refs` (cpp.rs:36-43).
- **Calling through a function-pointer variable** (`f()`, `fn()`, `lam()`) goes to `external` with `no-candidate-in-file`, or, when a function of that name exists, binds to it wrongly (§07).
- **Construction by declaration makes no edge.** `lib::Engine e;`, `Panel p;` and `std::unique_ptr<Shape> up(new Shape())` create no constructor edge (fixture f3). Only the inner `new` binds (fixture f2, L45).
- **Base and member initializers** (`Derived() : Vec(1)`) are not calls (fixture f13: there is no `Derived::Derived -> Vec::Vec` edge). An initializer *containing* a call, such as `v(compute())`, is recorded.
- **`std::make_unique<T>()`/`make_shared<T>()`** go to the stdlib bucket only, with no constructor edge (fixtures f2 and f4).
- **CUDA launches** `k<<<a,b>>>(…)` parse as comparisons and produce no edge. This is why `__global__` is a root (cpp.rs:2700-2712; rules.rs:1381-1384).
- **Paren-less calls and `value_ref`/method-reference records** do not exist in C++ (the plugin never sets `paren_less` or `value_ref`).

## 04. Imports & module resolution

**Supported:**

- **`#include "x.h"`** becomes an import of kind `include` (cpp.rs:681-701). Test: `include_directive_captured` (cpp.rs:2479). The cross-file resolver follows the transitive include closure to depth 8 (`collect_include_defs`, cross_file.rs:~1790-1805). Fixture f3: `main.cpp` includes `lib/engine.hpp`, and `e.start(1)` gives `Engine::start`, high (`cross-file:imports`).
- **`#include <x.h>`** becomes kind `system-include` (cpp.rs:702-717). It is followed only when it is not a platform header (`is_platform_header`). This handles `-I` layouts such as `<spdlog/spdlog.h>` (cross_file.rs:1808-1826). System includes are also the detection input for framework rules (§10).
- **Include-closure scoping.** When several family callables carry the name, a call bound through the include closure reaches only copies in the calling file or its closure files (`c_include_copies`, cross_file.rs:7259; used at 5717-5735). C++ files reach C definitions through an included C header (cross-language within the family, lookup retry under `language_family`, cross_file.rs:7246-7252). Tests: `cpp_file_resolves_call_to_c_definition_via_include` (cross_file.rs:10888), `cpp_include_does_not_import_every_c_function` (11004), `cpp_header_declaration_merged_into_a_body_elsewhere_still_binds_through_include` (cgg/tests/resolve.rs:770) and `cpp_free_function_declared_in_several_programs_binds_to_its_own_declaration` (resolve.rs:800).
- **Namespace aliases.** `namespace fs = std::filesystem;` becomes an import of kind `namespace-alias` (cpp.rs:209-235). `heads.rs` reads it to expand the head of a qualified receiver before the external check (heads.rs:625-631, 700-705). Fixture f3: `fs::exists` is `external`.
- **Type aliases.** `using X = T<…>;` and `typedef T X;` (plain name only) are recorded as `type_aliases` (cpp.rs:236-266) and used for typing (§05). They also add names to the workspace scope vocabulary (heads.rs:172-176).
- **External-namespace refusal** (`external_head`, heads.rs:660-720). cpp is in the language list. A path receiver whose scope segments the tree never declares is recorded as `external-module`. The declared scopes come from callable QNs that are true scope paths, field owners and type aliases (`cpp_scopes`, heads.rs:168-205). Fixture f3: `boost::split` is `external`. Tests: `cpp_namespace_declared_nowhere_is_external` (cross_file.rs:11524) and `cpp_macro_namespace_member_reached_through_an_include_is_kept` (11561).
- **Escape for macro-opened namespaces.** A namespace opened through a macro (`namespace KWSYS_NAMESPACE {`) whose member is declared by the include closure keeps the pre-refusal reading (`declared_by_include`, cross_file.rs:5341-5347).
- **Bare calls are never refused as external** in C++ (heads.rs:681-683: `lang == "cpp"` returns `None` for an empty receiver).
- **Scope-qualified binding (intra-file).** `ns::f()` binds only to a definition declared in exactly that scope path, with anonymous namespaces transparent, or to the constructor `ns::K::K`. It never binds to a member of a class inside the scope (intra_file.rs:515-575). Tests: `cpp_scope_qualified_call_needs_a_scope_path_target` (intra_file.rs:1753) and `cpp_scope_qualified_call_reads_the_real_scope` (1787).
- **QN carries the module.** C++ QNs are anchored for exact multi-segment lookup (`qn_carries_module`, cross_file.rs:4771-4777), and `unique_owner_names` keys C++ owners by QN path (cross_file.rs:4411).

**Not supported or partial:**

- **`using namespace X;` and `using X::f;`** are not recorded as imports; no `using_directive` arm exists in cpp.rs. Bare calls still resolve by name. Fixture f2, L25: `bare()` under `using namespace geo` binds `geo::bare`, high, because only one exists. Fixture f3: `using lib::util; util(3)` binds `lib::util`, high.
- **Namespace aliases to workspace namespaces do not narrow.** In fixture f3, `bl::util(4)` with `namespace bl = lib;` bound **both** `lib::util` and the unrelated global `util` in `other/unrelated.cpp`, at medium. The alias is consulted only for the external check.
- **Calls are not limited to the include closure when nothing in the closure declares the name.** Fixture f3: `unrelated()` binds `other/unrelated.cpp::unrelated` at medium, although `main.cpp` never includes it.
- **Not applicable:** `FileFacts::packages`, manifest reading (no CMake/compile_commands/`conanfile`/vcpkg) and `not-in-imported-module` (`modules.rs` has no C++ branch). The cmake plugin is a separate language.
- **C++20 modules** (`import foo;`, `export module`) are not handled (no arm in cpp.rs).

## 05. Type inference & receiver typing

**Supported:**

- **Parameters.** Parameter types are parsed from `signature_hint` (`collect_param_types`, type_hints.rs:1414+). Pointer and reference parameters use the nominal type: in `const Pump& p`, `Pump* pp`, `Motor m`, the receiver type is the class. Test: `cpp_pointer_and_reference_params_use_the_nominal_type` (type_hints.rs:3149). Fixture f4, L7-12: all high.
- **Locals and fields.** `record_local`/`record_field` (cpp.rs:509-577) record `nominal_type`. That is the last scope segment with template arguments, cv, `typename`, `*` and `&` stripped, and **only capitalised names** (cpp.rs:1211-1222). `int`, `string` and `vector` are therefore never types. Locals are scoped by `scope_byte`, and C++ locals are looked up per call site (CLAUDE.md, type_hints step 1). Tests: `local_variable_and_field_types_are_recorded` (cpp.rs:2051), `cpp_typed_local_and_field_chain_resolve` (cgg/tests/resolve.rs:563) and `cpp_same_local_name_in_two_functions_keeps_each_type` (628). Fixture f4, L15-16: high.
- **Template pointee.** For `std::unique_ptr<Motor> um`, `shared_ptr<const Motor>` and `sk_sp<T>`, the variable is its own template type (`unique_ptr`, which is dropped as lowercase) and also has `name#deref`, set to the first type argument, used only for `->` calls (`pointee_type`, cpp.rs:1227-1231; `DEREF_SUFFIX`). Test: `a_template_field_or_local_is_its_own_type_and_derefs_to_its_argument` (cpp.rs:2109). Fixture f4: `um->spin()` and `p->sm->spin()` are high.
- **Field chains.** In `p->motor->spin()` and `this->x->y()`, each segment is typed through `FileFacts::field_types`, run-wide (`member_chain_type`, type_hints.rs:579-591). Test: `cpp_field_chain_rewrites_to_the_field_type` (type_hints.rs:3188). Fixture f4, L22-23: high.
- **Object-like macros.** `#define THEKERNEL Kernel::inst` is recorded as a `macro_alias` (cpp.rs:658-679). The receiver `THEKERNEL->pump->spin()` is expanded and typed (type_hints.rs:640-650). Tests: `object_like_macros_and_static_fields_are_recorded` (cpp.rs:2155), `cpp_object_like_macros_type_as_their_expansion` (type_hints.rs:3313) and `cpp_object_like_macro_receivers_resolve` (resolve.rs:672). Fixture f4, L25: high.
- **Type aliases.** `using MotorAlias = Motor;` and `typedef Pump PumpT;` are resolved run-wide, first definition wins, chains up to 8, with builtins dropped (`type_alias_index`, type_hints.rs:111-140; rewrite at 928-931). Test: `cpp_receiver_typed_through_an_alias_reaches_the_aliased_class` (resolve.rs:829). Fixture f4, L11-12: high.
- **Casts and constructions** as receivers are covered in §03.
- **Untyped fallback.** A C++ typed receiver whose owner lookup finds nothing is retried with the receiver as written, at medium (`untyped_receivers`, type_hints.rs:933-943; cross_file.rs:2510-2545). Test: `cpp_typed_receiver_with_no_known_class_falls_back_to_the_untyped_guess` (resolve.rs:857).
- **Generic parameter types.** A generic parameter is its outer type (`Map<String, Handler> h` → `Map`), and the parameter list is located structurally, skipping `__attribute__((…))` and `decltype(…)` (CHANGELOG.md:340-351).

**Not supported:**

- **`auto` locals are never typed** (the type text `auto` is lowercase, so `nominal_type` returns `None`). Fixture f4:
  - `auto a = make_motor(); a.spin()` (L17) fans out to **both** `Motor::spin` and `Pump::spin` at medium.
  - `auto b = Motor()` (L18) and `auto c = std::make_shared<Pump>()` (L19) do the same.

  Return-type propagation therefore does not reach C++ locals. The CLAUDE.md "return types" claim applies to other languages.
- **Method return types are not used for chained calls** (fixture f4, L24: `p->get().spin()` fans out).
- **Container elements and range-for variables are untyped.** In fixture f4, `v[0].spin()` and `for (auto& x : v) x.spin()` both fan out (L20-21).
- **Static-field receivers through a path** are unresolved. Fixture f4, L26: `Kernel::inst->pump->spin()` is unresolved, though the macro spelling of the same chain resolves.
- **Template parameter receivers** (`static_cast<D*>(this)->impl()`, CRTP) are unresolved (fixture f5, L18: `impl` with receiver `D`).
- **Not applicable to C++:** `declared_type` nullability, maybe-wrappers, scoped closure parameter types (`closure_types.rs`), implicit receivers and `weak_receiver_type`. The plugin sets none of them.

## 06. Inheritance, dispatch & overloading

**Supported:**

- **Bases.** `base_class_clause` is read through `attrs::base_types` (cpp.rs:163; attrs.rs:173) onto every method's `base_types`. Test: `bases_and_virtual_are_recorded_on_methods` (cpp.rs:2215).
- **Inherited members through a typed receiver.** `l->plain()`, where `Leaf : Mid : Base`, gives `Base::plain`, high (fixture f5, L24-25). This uses the base-chain retry (`resolve_via_bases`, CLAUDE.md step 3).
- **Bare call inside a member.** The enclosing class is searched first (intra_file.rs:440-470); this is the C++ unqualified-lookup rule. Test: `cpp_bare_call_in_a_member_prefers_the_enclosing_class` (resolve.rs:950). Fixture f11: `S::run(){ process(); }` gives `S::process`, high, over the global `process`. Inherited bare calls also work: in fixture f5, `plain()` inside `Leaf::tick` gives `Base::plain`, high.
- **Explicit base call.** `Base::draw()` and `this->Mid::draw()` reach exactly that class (fixture f5, L10/L15: high).
- **Virtual dispatch** (`cpp_virtual_dispatch`, cross_file.rs:2898-3200, resolver `cross-file:cpp-virtual`). For each resolved edge whose target slot is virtual (declared `virtual` anywhere up the base chain, depth 8), the pass adds a `Via::Dynamic` low edge to every same-named override below the owner, uncapped. Destructors and constructors are excluded.
  - It runs **only under `--dynamic-dispatch`**, which `--dead-code` forces on (cross_file.rs:2899-2905).
  - Note: the function's doc comment at cross_file.rs:2915-2918 still says the edges "stay in the default graph". That is stale against the gate at 2902.
  - Tests: `cpp_virtual_call_reaches_overrides` (resolve.rs:1038), `cpp_virtual_overrides_are_not_capped` (1089), `cpp_non_virtual_call_does_not_fan_out_to_overrides` (1139), `cpp_typed_derived_pointer_does_not_reach_sibling_overrides` (1194) and `cpp_virtual_dyn_edges_are_opt_in_behind_dynamic_dispatch` (1244).
  - Fixture f5 with `--dynamic-dispatch`: `use -> Mid::draw/Leaf::draw` and `use -> Mid::tick/Leaf::tick` are low dynamic.
- **Member-pointer tables.** `(m->*table[i])()` reaches every `&Type::m` in `table`, plus their overrides (same pass). Test: `cpp_member_pointer_table_reaches_overrides`.
- **Arity-based overload narrowing.** cpp is in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583). Defaults, varargs and packs are handled by `accepts_arity`. A one-file, one-QN overload set is one callee (`binds_same_name_set`, intra_file.rs:836-850) and is exempt from the fan-out cap, binding at medium when several overloads survive. Fixture f6:
  - `l.write(1)` binds `write(int)` and `write(const char*, int=0, int=1)`, both medium.
  - `l.write(1,2)` binds two overloads, medium.
  - `l.write("x",1,2)` binds the three-parameter overload only, high.
  - `l.fmt(1,2,3,4)` (a pack) and `l.va(1,2,3)` (varargs) are high.
  - `f(1)` and `f(1.0,2.0)` each bind one overload, high.
  - `g(1)` with a default argument is high.
- **Typed calls reach every same-QN definition**, for example a real body and a test stub (`several`, cross_file.rs:6146-6160). Test: `cpp_typed_call_reaches_every_definition_not_only_the_last` (resolve.rs:972).
- **Template specializations.** A typed lookup over several `Name<args>` owners binds the one written exactly, otherwise it is refused over the cap (`narrow_specializations`, cross_file.rs:4906).

**Not supported or partial:**

- **Overloads are not distinguished by argument type** (fixture f6, `write(1)` as above).
- **Inheritance fan-out and dead-code override evidence do not cover C++.** cpp is **not** in `INHERITANCE_LANGS` (dispatch.rs:110-124; the comment at 103-109 explains that C++ uses `cpp_virtual_dispatch`). So `inheritance_fanout` and the dead-code `overrides_below` evidence do not run for C++ (deadcode/mod.rs:342).
- **`--dynamic-dispatch` `fanout`** (`trait_impl_target`) does not apply. The plugin never sets it, despite declaring the `impls` signal (cpp.rs:40). Test: `cpp_and_rust_are_left_to_their_own_passes` (dispatch.rs:780).
- **`override` and `final`** are not recorded as attributes (`push_def` only adds the CUDA qualifiers and `virtual`, cpp.rs:333-337). `implements-external-contract` therefore never fires (§12).
- **A qualified non-virtual call still gets virtual fan-out.** In fixture f5 with `--dynamic-dispatch`, `Base::draw()` inside `Mid::draw` produced `Mid::draw -> Leaf::draw` (low dynamic). The seed loop (cross_file.rs:3043-3093) does not check whether the site was written with an explicit qualifier.
- **Not applicable:** `super`/`base` keywords (C++ has neither) and `super-base-out-of-graph`.
- **Not supported:** CRTP (fixture f5), extension methods, and mixins or traits (none exist in C++).

## 07. Scoping & shadowing

**Supported:**

- **Per-function local types.** Local *types* are scoped by declaration byte (`scope_byte`, cpp.rs:555), and C++ uses per-call-site lookup (CLAUDE.md step 1; test `cpp_same_local_name_in_two_functions_keeps_each_type`).
- **Anonymous namespaces** are per translation unit. A body in `(anonymous)` absorbs a prototype only in the same file (cpp.rs:1388-1393).
- **Class-first bare lookup** inside members (§06).

**Not supported:**

- **No `FileFacts::locals` for C++.** `locals.rs` has no cpp spec (locals.rs:382-391 lists js/ts, python, lua, scala, kotlin, swift, rust, dart, ocaml and zig). So `shadowed-by-local` never fires, and a parameter or local named like a function is bound to that function. Fixture f11:
  - `void caller(void (*handler)()) { handler(); }` binds the global `handler()` at **high**, which is a wrong edge at high confidence.
  - `auto process = []{}; process();` is `ambiguous-in-file` between the global `process` and `S::process`, instead of being recognised as the local.
- **No lexical narrowing of nested definitions** (C++ is not in the list; CLAUDE.md step 3), and no positional shadowing.
- **Conditional definitions.** `#if`/`#else` twins both become nodes and both bind, at medium (fixture f7c). There is no `DefRecord::conditional` for C++.

## 08. Macros, preprocessor & codegen

**Supported:**

- **Function-defining `#define` expansion** (`c_macro.rs`).
  - Invocations at file, namespace or class scope of the last same-named macro earlier in the file, or the first one in the `#include "…"` closure (depth 8, c_macro.rs:36), are expanded one level, with `#`, `##` and `__VA_ARGS__`/GNU comma handled (c_macro.rs:483-575).
  - The expansion is re-extracted with the C++ walker under the invocation's scope and classes (`cpp::extract_fragment`, cpp.rs:81-99; c_macro.rs:582-609).
  - Only function definitions that parse cleanly are kept. They are tagged `macro:<NAME>` (c_macro.rs:1-25).
  - Tests: `cpp_class_body_macro_generates_methods` (c_macro.rs:1181), `header_macro_generates_functions_in_the_including_file` (1071), `pastes_and_stringizes` (998) and `variadic_arguments_and_gnu_comma` (1014).
  - Fixture f7b: a header macro used at file, class and namespace scope yields `get_a`, `Panel::get_b`, `Panel::on_click` and `ns::on_key`, all tagged `macro:…`, and all called at high.
- **Function-like macro nodes.** Every `#define NAME(…)` is a callable tagged `macro` (cpp.rs:627-656). A call to it binds (fixture f7: `caller -> SQUARE`, high, cross-file through the include).
- **Object-like macros** are used only to type receivers (§05).
- **Item-position invocations** become dead-code roots (`toplevel:macro-invocation`, roots.rs:616-660; fixture f9: `DEF_GETTER`, `REGISTER`).
- **Identifier arguments of unexpanded invocations** go to `name_mentions`, which only the dead-code report reads (CLAUDE.md). Fixture f9: `registered`, passed only through `REGISTER(registered);`, is reported with the evidence `name-collides-with-screened-site` at medium.
- **Templates** are parsed as their bodies (§02). Template arguments are stripped from call names (§03) and from QNs for unification.

**Not supported or partial:**

- **Single-level expansion only.** There is no nested or rescanned expansion, and macros used inside function bodies are never expanded (c_macro.rs:9-17). In fixture f9, `REGISTER(fn)` expands to a static initializer, not a function, so the `register_fn` call inside it is lost and `register_fn` is reported dead.
- **No `#if` evaluation** (§01). Both branches, and `#if 0` blocks, are extracted (fixture f11: `disabled`).
- **Misparse can corrupt the rest of the file.** In fixture f7, two consecutive semicolon-less macro invocations in a class body after a data member (`int b_; DEF_GETTER(b) DECL_HANDLER(click)`) caused the following:
  - a phantom method `DECL_HANDLER`;
  - no `Panel::get_b`;
  - every later free function in the file reclassified as kind `method`;
  - a lost `ns::on_key`.

  Each macro alone works (fixtures f7b and f7c/a).
- **README is stale.** README.md:1459 says "C/C++ function-like macros are extracted as callables but not expanded", which contradicts the `c_macro.rs` expansion (CHANGELOG.md:1019-1034).
- **Not applicable to C++:** quote/unquote.

## 09. FFI & cross-language links

**Supported:**

- **C/C++ ↔ asm** (`ffi::link_ffi` pass A, ffi.rs:36-175, C family via `same_family`). Fixture f10:
  - `call_asm() { asm_func(1); }` gives `asm_func` (asm label), medium `ffi:c-asm`.
  - The asm `call c_api` gives `c_api` (cpp), medium `ffi:asm-c`.
  - The same call also binds the C++ `extern "C"` prototype at high (intra-file), because the prototype stays a node when no body exists.
  - Value and string references are skipped (CLAUDE.md step 4).
- **C ↔ C++ ↔ Obj-C by include.** These are same-family name lookups, not FFI edges (§04; CHANGELOG.md:1894).
- **Protobuf rpc → C++ implementation** (`descriptor::link_descriptors`, descriptor.rs:60-140). This is language-agnostic: the owner name must contain the service name. Fixture f14: `hello.Greeter.SayHello -> GreeterServiceImpl::SayHello`, low `descriptor:grpc`.

**Not supported:**

- **`extern "C"`, `JNIEXPORT`, `__declspec(dllexport)` and `__attribute__((visibility("default")))`** are not recorded as attributes. `classify_ffi` needs `extern:c`, `jniexport` or `dllexport` attributes (ffi.rs:643-676), so C++ exports are never `ffi:export` roots and get no peer edges. Fixture f9: `extern "C" void c_export()` is reported dead, medium.
- **JNI.** A Java `native` method has no link to `Java_com_x_Native_compute` in C++ (fixture f10: no edge). Pass B links exports only, and the Java `native` attribute is an import (ffi.rs:670).
- **Python `ctypes`/`cffi` → C++.** Not linked (fixture f10: `lib.c_api()` is external, and the bare `c_api()` is unresolved).
- **pybind11/nanobind/Boost.Python, napi/node-addon-api, wasm (Emscripten `EMSCRIPTEN_KEEPALIVE`) and P/Invoke targets.** There are no C++-side rules or attributes. A grep of the plugin finds none.
- **Not applicable to C++:** markup code-behind.

## 10. Frameworks & entry points

Rules with `language: "cpp"` (rules.rs). Matching is per exact language, with no family sharing (frameworks/mod.rs:82-111), so the `c-*` rules do not apply to C++ files.

| rule id | kind | node | trigger | evidence |
| --- | --- | --- | --- | --- |
| `cuda` | Lifecycle | true | attribute `__global__`, read from the declaration head by `cuda_qualifiers`; no import gate | rules.rs:1377-1397; fixture f8: `saxpy` in `kern.cpp` gives `<framework-entry>::lifecycle::cuda::saxpy -> saxpy` (low `framework_entry:cuda`); test frameworks/mod.rs:3093 |
| `aws-lambda` | Network | true | detect `aws/lambda-runtime/runtime.h` (system include); registrar `run_handler` (shape B, identifier argument) | rules.rs:4598-4614; fixture f8: `<framework-entry>::network::aws-lambda::my_handler -> my_handler`; cgg/tests/frameworks.rs:1210 |
| `gtest` | Test | true | detect `gtest`, `gmock`, `gtest/gtest.h`, `gmock/gmock.h`; **no matchers**. It is reported in `seen_no_rules` with gap prose | rules.rs:7879-7895; fixture f8 coverage `seen_no_rules: gtest` |
| `cpp-test-macros` | Test | true | detect doctest/Catch2 headers; **no matchers** (gap: `TEST_CASE("…")` bodies are not callables) | rules.rs:7896-7918; fixture f8 `seen_no_rules` |
| `cpp-runtime` | Lifecycle | false | detect `signal.h csignal stdlib.h cstdlib pthread.h setjmp.h stdio.h`; **no matchers**. A disclosure rule that points at `cpp-callbacks` and lists uncovered shapes | rules.rs:7919-7942; fixture f8 `seen_no_rules: cpp-runtime` |
| `cpp-callbacks` | Lifecycle | false (root marks only) | `detect_calls` and registrars `signal atexit at_quick_exit on_exit pthread_create pthread_once pthread_key_create qsort bsearch call_once`; only identifier arguments that name in-tree functions | rules.rs:8012-8045; fixture f8: `on_sig`, `on_exit_hook`, `thread_main`, `cmp` and `once_fn` are rooted as `framework-callback` with rule `cpp-callbacks:registrar` (5 root marks) |

**Helper tables:**

- `member_sources_for`, `signature_methods_for`, `container_attributes_for`, `markup_kinds_for`, `visibility_entries_for` and `self_module_markers_for` have **no cpp entries** (rules.rs:11666-11860, grep). Their entries are only for javascript, typescript, solidity, java, csharp and rust.
- `TEST_CLASS_MEMBERS_ARE_ROOTS` is python, php and ruby only (rules.rs:11806).

**Registrar verbs active for C++:** `run_handler` and the ten `cpp-callbacks` verbs. User-supplied verbs can be added through `ExtractCtx` (the registrar gate, registrar.rs:433-440).

**Built-in program entries:** `main`, plus `_start`, `WinMain` and `DllMain` (roots.rs:21-29).

**Not covered** (all from the `cpp-runtime` gap text, rules.rs:7940, plus fixtures):

- Qt (`connect`, slots, `Q_INVOKABLE`). Fixture f8: `onClick` was lost and `Win::wire` is reported dead.
- `std::thread`/`std::async`.
- Algorithm comparators (`std::sort`).
- `std::function`/`std::bind` stored targets.
- `__attribute__((constructor/destructor))`. Fixture f9: `init_ctor` is reported dead.
- Static-object constructors.
- `sigaction`.
- Interrupt vectors.
- Unreal (`UFUNCTION`), Boost.Asio handlers, gRPC C++ service registration (only the descriptor link in §09), Crow/Drogon/Pistache/cpprestsdk routes, Google Benchmark, libFuzzer `LLVMFuzzerTestOneInput`, ROS callbacks and Arduino `setup`/`loop`. There are no rules for any of these (grep of rules.rs for `language: "cpp"` finds only the six rules above).

## 11. Test harness entries

**Supported:**

- **Name convention.** `is_test_name` uses the default arm for cpp: a name starting with `test_` or `Test` (roots.rs:975-993). It is **not gated** on the file being a test file. Fixture f12: `test_named_in_src` in `src/` is rooted as `test-entry` by `builtin:test-name`. Fixture f8: `test_helper_named` is rooted the same way.
- **Test-file classification** (`testfile.rs:111-117`): `*_test.c/.cc/.cpp`, `test_*` and `*_unittest.*`, plus the generic test directories. This drives the `--include-tests` report scope.

**Not supported:**

- **gtest.** `TEST`, `TEST_F` and `TEST_P` bodies become callables named `TEST`/`TEST_F` with no test root. `SetUp`/`TearDown` are not rooted. Fixture f8 dead-code: `TEST`, `TEST_F`, `Fix::SetUp`, and `under_test` (called only from them) are **all reported dead at medium**.
- **Catch2/doctest.** There is no callable for the test body. In fixture f8, the `catch_target()` call in `TEST_CASE("adds")` became a `toplevel:invocation` root by accident, because the misparse leaves it at file scope.
- **No other harness has a rule:** Google Benchmark (`BENCHMARK(fn)`), Boost.Test, CppUnit, libFuzzer (`LLVMFuzzerTestOneInput`) and QTest. No kind-`test` rule mints a harness node for C++.
- **No `test_defs` signal** (cpp.rs:36-43), so there is no `TestRole`. The dead-code capability table reports `test_tagging: none` (fixture f8 `capabilities`).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** does not include cpp.

## 12. Dead-code analysis

**Capability row** (fixture f8 `--dead-code` JSON):

```text
language cpp: class degraded; visibility none; attributes full; value_references full*; dispatch full; exports none; test_tagging none; max_confidence medium
```

\* `value_references` is observation-based. It showed "full" in f8, where registrar refs existed, but fixture f9 findings carry `language-lacks-value-references`.

**Roots that fire for C++:**

- `builtin:main` (`main`, `_start`, `WinMain`, `DllMain`).
- `builtin:test-name`.
- `framework:entry-node` (cuda, aws-lambda).
- `framework:rule` (cpp-callbacks).
- `toplevel:invocation` (calls in global initializers; fixture f13 `compute`).
- `toplevel:macro-invocation` (roots.rs:616-660; fixture f9).
- `user:roots`.

`ffi:export`, `lang:export`, `builtin:lifecycle` and `builtin:framework-attr` never fire for C++:

- No FFI attributes are recorded.
- There are no exports.
- `LIFECYCLE` has no cpp entry (roots.rs:95-157).

**Signals extracted:**

- CUDA attributes and `virtual`.
- `unreachable` regions after a terminator (`cfg::unreachable_after_terminator`, cpp.rs:69-72, under `deadcode_signals`). Note: outside tests, a grep found no consumer of `FileFacts::unreachable` regions, only the capability flag (cgg/src/lib.rs:1935), and fixture f12 `regions` was empty. Treat this as unverified for impact.
- `name_mentions` from macro arguments.
- Member-pointer takes, which go only to dispatch.

**Confidence caps and evidence seen for C++:**

- `language-lacks-visibility`: no access specifiers, `static` or anonymous-namespace visibility recorded (cpp.rs:346). Every C++ finding is capped at **medium** (fixtures f8 and f9: all findings medium).
- `macro-generated` (`macro:<name>`). Fixture f9: `get_unused` carries `('macro-generated','DEF_GETTER')`.
- `implicitly-invokable-kind` for constructors. Fixture f9: `Ext::Ext`.
- Destructors are never candidates (`is_candidate`, deadcode/mod.rs:598-601). Fixture f9: `~Ext` is absent from the findings.
- `name-collides-with-screened-site` (fixture f9: `registered`).
- Virtual overrides reached by `cpp-virtual` low edges count as reached, because `--dead-code` forces dynamic dispatch. Fixture f9: `Impl::go` is not reported.

**Not supported:**

- **`implements-external-contract`.** It needs override markers (`@Override`/`override`/`@override` in attributes, deadcode/mod.rs:409, 502-505), and C++ `override` is not recorded. Fixture f9: `Ext::paintEvent() override` (base `QWidget`, outside the tree) is reported dead, medium, with no contract evidence.
- **`overrides_below`/`ImplementedTraitSurface`** evidence: cpp is not in `INHERITANCE_LANGS` (deadcode/mod.rs:342).
- **No visibility-based rules.** `static`, file-local and anonymous-namespace functions are treated like extern ones (fixture f9: `file_local_unused` and `(anonymous)::anon_unused` are both medium).
- **`reflection` cap.** Not applicable, because there is no dynuse for C++ (§15).
- **False dead findings in fixtures:**
  - `only_by_ptr`: taken by address in a table, and also `&only_by_ptr` (fixture f9).
  - `extern "C"` exports (`c_export`).
  - `__attribute__((constructor))` (`init_ctor`).
  - gtest bodies (fixture f8).
  - Qt slot callers (`Win::wire`).

## 13. Builtins & stdlib tables

**Supported:**

- **`crates/cgg-core/src/stdlib/cpp.txt`**, 553 lines. This is the stdlib name manifest, registered at stdlib.rs:233. It classifies unresolved sites into the `stdlib` bucket. Fixture f2: `static_cast`, `std::make_unique` and `std::sort` are `stdlib_calls`. Fixture f8: `signal`, `atexit`, `pthread_create`, `qsort`, `std::call_once` and `connect` are `stdlib`. (C has its own 278-line `c.txt`.)
- **`CAST_INTRINSICS`** for cpp: `static_cast`, `dynamic_cast`, `reinterpret_cast`, `const_cast` (stdlib.rs:35-44).
- **Construction by call:** cpp is in `constructs_by_call` (receiver.rs:46-51), but a C++ `Foo::Create()` path is deliberately not a construction (receiver.rs:274-286).
- **`CPP_STDLIB_HEADERS`**, a detection-only list of about 100 header names (detect.rs:332-445).
- **`is_platform_header`** stops system includes from being followed into vendored libc copies (cross_file.rs:1808-1817).
- **`--include-stdlib` / `--include-external`** mint `<stdlib>::…` and `<external>::…` leaf nodes. Fixture f12 mermaid: `<stdlib>::printf`, `<stdlib>::std::sort`, `<stdlib>::puts` and `<external>::boost::split`, with `std`/`ext` arrow labels.

**Not supported:**

- **`CONSTRUCTING_METHODS`** has no cpp entry (ruby only, stdlib.rs:58).
- **No smart-pointer or wrapper table** like `rust_types`. Template pointee is generic: it takes the first type argument of any template (§05).
- **No implicit-package table.** Not applicable.

## 14. Confidence & certainty

**Can be `high`:**

- Intra-file single candidates (fixtures f1, f2, f5 and f6).
- Typed receivers from parameters, locals, fields, casts, constructions, aliases and macros (fixture f4).
- Calls through the include closure to a unique definition (fixture f3: `main -> Engine::start` and `main -> lib::util`, high `cross-file:imports`).
- Inherited members through a typed receiver (fixture f5, L24-25).
- An overload set narrowed by arity to one (fixture f6).
- The general `CertainCtx` rules (CLAUDE.md "high means structurally certain").

**Forced to `medium`:**

- By-name and duck fan-out (fixture f4, `auto` locals).
- More than one surviving overload (fixture f6).
- Several same-QN copies (preprocessor twins, fixture f7c).
- The C++ untyped-receiver fallback (cross_file.rs:2510-2515).
- Static-field chains (fixture f3, L10: `lib::Engine::instance->stop()`, medium).
- Namespace-alias calls (fixture f3, `bl::util`).
- Calls to files outside the include closure (fixture f3, `unrelated`).

**`low`:**

- `cpp-virtual` dynamic edges.
- Framework entry edges.
- Descriptor links.

**Mermaid `?` marker.** It is generic: an arrow with no `high` site gets a leading `?` (fixture f8: `-->|?ref|`).

**Known high-confidence wrong edge:** a bare call through a function-pointer parameter binds a same-named function at high (fixture f11: `caller -> handler`, high), because there are no locals (§07).

The CHANGELOG reports precision of 30/30 on sampled C++ high-promotion edges, and a C++ medium share of 71.6% → 60.4% (CHANGELOG.md:73-81).

## 15. Dynamic use & reflection

- **No `dynuse` coverage.** `dynuse.rs` `REFLECTIVE` lists python, ruby, java, csharp, javascript, typescript and php (dynuse.rs:24-58). Only the java, go, php, ruby, javascript, typescript, csharp and python plugins call `dynuse::extract` (grep). cpp.rs does not call it and does not declare `dyn_uses`.
- **Not handled:** `dlsym`/`GetProcAddress` lookups by literal name, `QMetaObject::invokeMethod(obj, "slot")`, RTTI `typeid`-based dispatch and plugin factories. A grep of the cpp plugin finds no handling.
- **`std::function`/function-pointer dispatch** is unresolved (§03).
- **Member-pointer tables** are the only modelled dynamic call. They are handled through `cpp_virtual_dispatch` under `--dynamic-dispatch` (§06).

## 16. Known gaps & limitations

**Detection and extraction:**

- **Detection:** `.cu`/`.cuh`/`.inl`/`.tpp`/`.ipp`/`.ino`/`.c++` are not detected (fixtures f8 and f11). The CUDA rule therefore cannot see `.cu` kernels.
- **Missing callables:**
  - lambdas;
  - conversion operators (`operator bool`);
  - explicit specializations of free function templates;
  - doctest/Catch2 test bodies;
  - methods in Qt `slots:` sections lost to misparse (fixtures f1, f8 and f13; rules.rs:7915).
- **Wrong names or kinds:**
  - namespace free functions get kind `method` (cpp.rs:393-399; fixture f1);
  - inline `friend` functions are owned by the class;
  - out-of-class friend bodies are never unified with the in-class friend declaration (fixtures f1 and f13);
  - gtest tests all share the QN `TEST`/`TEST_F`.

**Calls, imports and typing:**

- **Unrecorded calls:**
  - operators;
  - call operator `obj()`;
  - constructions by declaration;
  - base/member initializers;
  - function values outside registrar verbs (`std::thread`, `std::function`, `&fn` in tables);
  - CUDA launches;
  - `::ns::f()` (fixtures f2, f3 and f13).
- **No locals or shadowing:** a function-pointer parameter can bind a same-named function at high (fixture f11).
- **Typing gaps:**
  - `auto` locals;
  - method and function return types;
  - containers and range-for;
  - static-field path receivers;
  - template-parameter (CRTP) receivers (fixtures f4 and f5).
- **Imports:**
  - `using namespace` and `using X::f` are not recorded;
  - namespace aliases do not narrow workspace calls;
  - calls bind by name to files outside the include closure at medium (fixture f3);
  - C++20 modules are unsupported.
- **Preprocessor:**
  - no `#if` evaluation, so `#if 0` code is extracted (fixture f11);
  - macro expansion is one level and item position only (c_macro.rs:9-17);
  - consecutive semicolon-less class-body macros can derail parsing of the rest of the file (fixture f7).
- **Duplicate prototypes:** when the same free function has bodies in two or more other files, the header prototype is kept, and calls bind to the body-less prototype (cpp.rs:1397-1412; fixture f11: `ua -> a/dup.h::dup`, high, and the body in `a/dup.cpp` gets no caller).

**Dispatch and dead code:**

- **Dispatch:**
  - the virtual fan-out also fans out an explicitly qualified (non-virtual) call (fixture f5: `Mid::draw -> Leaf::draw`);
  - `override`/`final` are not recorded;
  - cpp is not in `INHERITANCE_LANGS`;
  - the stale doc comment at cross_file.rs:2915-2918 says the virtual edges are in the default graph, but they are gated at 2902.
- **FFI:** no `extern "C"`/`JNIEXPORT`/`dllexport` attributes, so there are no export roots and no JNI, ctypes or pybind11 links (fixtures f9 and f10).
- **Frameworks and tests:**
  - gtest and doctest/Catch2 are detected but have no matchers (gap prose, rules.rs:7892 and 7915);
  - Qt, `__attribute__((constructor))`, `std::thread` and the other shapes listed in the `cpp-runtime` gap (rules.rs:7940) are not covered;
  - there is no benchmark or fuzz harness rule.
- **Dead code:**
  - every finding is capped at medium (no visibility);
  - there is no `implements-external-contract` evidence;
  - `static` and anonymous-namespace functions are not treated as file-local;
  - the `unreachable` regions are extracted but no consumer was found (fixture f12).

**Documentation and tests:**

- **Docs:** README.md:1459 ("function-like macros … not expanded") is stale against `c_macro.rs`. The README C++ row (README.md:494) is otherwise consistent.
- **Ignored tests:** a grep of crates/ for `#[ignore` lines mentioning cpp or C++ finds none.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | partial | 7 extensions plus `.h` content and sibling sniffing; `.cu`/`.cuh`/`.inl`/`.tpp`/`.ino` are not detected; no `#if` evaluation |
| 02 Callables extracted | partial | functions, methods, ctors/dtors, operators, templates, out-of-line bodies and prototype unification are good; no lambdas or conversion operators; namespace functions have kind `method`; friends get the wrong owner |
| 03 Call-site forms | partial | bare, qualified, template, arrow/dot, casts, `new` and member-pointer calls work; operators, call operator, declarations/initializers, function values and `::ns::f` are missing |
| 04 Imports & module resolution | partial | transitive `#include` closure, external-namespace refusal and namespace-alias heads work; `using` is not recorded; aliases do not narrow; out-of-closure by-name binds |
| 05 Type inference & receiver typing | partial | params, locals, fields, chains, aliases, macros, casts and template pointee work; no `auto`, return types, containers or CRTP |
| 06 Inheritance, dispatch & overloading | partial | base chains, class-first lookup, arity overloads and virtual/member-pointer fan-out (opt-in) work; no override marker; qualified calls are fanned out |
| 07 Scoping & shadowing | minimal | no `FileFacts::locals`; a parameter or local can bind a same-named function at high |
| 08 Macros, preprocessor & codegen | partial | one-level function-macro expansion with `macro:` tags and object-macro typing; no `#if` evaluation; misparse risk |
| 09 FFI & cross-language links | minimal | asm↔C/C++ and protobuf rpc only; no `extern "C"`/JNI/ctypes/pybind11 |
| 10 Frameworks & entry points | minimal | cuda, aws-lambda and cpp-callbacks enumerate; gtest/doctest/cpp-runtime are disclosure-only; no Qt/Unreal/web frameworks |
| 11 Test harness entries | minimal | `test_`/`Test` name prefix only (ungated); gtest/Catch2 bodies are not rooted, and gtest bodies are reported dead |
| 12 Dead-code analysis | partial | class `degraded`, max confidence medium; macro and top-level roots work; no visibility, exports, FFI roots or external-contract evidence |
| 13 Builtins & stdlib tables | full | 553-name `cpp.txt`, cast intrinsics, stdlib header list, platform-header guard, include-stdlib/external nodes |
| 14 Confidence & certainty | partial | high for typed, include-anchored and arity-narrowed calls; one observed wrong edge at high (function-pointer parameter) |
| 15 Dynamic use & reflection | none | no dynuse; only member-pointer tables are modelled |
| 16 Known gaps & limitations | n/a | listed above |
