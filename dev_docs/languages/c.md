# c

The C plugin (`CPlugin`, `crates/cgg-lang/src/plugins/c.rs`, 612 lines including inline tests) parses `.c` and `.h` files with the crates.io grammar `tree-sitter-c` (`tree_sitter_c::LANGUAGE`, c.rs:35-37). Nothing is vendored. A single recursive `CWalker` (c.rs:86-384) does the following:

- Records `function_definition` bodies, prototypes (`declaration` with a function declarator) and function-like `#define`s as `FreeFunction` definitions.
- Records quoted `#include "…"` as `include` imports and `<…>` as `system-include` imports.
- Records `call_expression` sites whose callee is an identifier or a `field_expression`.
- Runs the registrar-argument capture for the C runtime callback verbs.

After the walk, the plugin collects item-position macro invocations (`c_macro::scan_invocations`, c.rs:56-57). Under `--dead-code` it also collects statements after a terminator (`cfg::unreachable_after_terminator(&C_LIKE)`, c.rs:58-61).

The plugin declares one signal only, `unreachable: true` (c.rs:28-33). It does **not** declare `visibility`, `attributes`, `exports`, `impls`, `value_refs`, `test_defs` or `dyn_uses`. There is no `resolver_kind` override, no `ts_language_for`, no `max_line_bytes` and no `shebangs`.

C-specific resolution is C-family code shared with C++ and Obj-C (`cgg_core::same_family(lang, "c")`, lang_family.rs:28). It lives in:

- `cgg/src/lib.rs:508-535`: function-macro expansion, then prototype unification.
- `cgg-resolve/src/cross_file.rs`:
  - the `include` / `system-include` closure (1789-1827, 3385+);
  - `c_include_copies` (7259-7360);
  - the language-family retry (7366+).
- `ffi.rs`: the asm↔C pass (36-186) and the `c-abi` source-language gate (698).
- `deadcode/roots.rs`: `LANG_ENTRY` (27) and the `toplevel:macro-invocation` rule (616-660).

Fixtures cited below are under `<scratch>/parity/c/` (f1 … f12). Each was run with `target/release/cgg` 0.11.0 using `-t json -o out.json`, and dumped with `show.py` in the same directory. `--dead-code` runs used `--dead-code-format json --dead-code-confidence low`.

## 01. Detection & parsing

- **Extensions `.c` and `.h`** (c.rs:25-27). Fixture f1: `detected_via: "extension:.c"`.
- **`.h` disambiguation** (`header_verdict`, detect.rs:224-261). It applies to any `.h`, whichever plugin matched the extension first (detect.rs:104-108, 118-120). The order is:
  1. **Content sniff of the first 8 KiB** (`has_cpp_content`, detect.rs:270-327), after comments and strings are stripped. The following make the file C++ (`header-content:cpp`):
     - a C++ stdlib include;
     - `namespace`, `template<`, `enum class`, a base clause or an access specifier;
     - `using`, `constexpr`, `noexcept` or `::`;
     - a `virtual` member;
     - a raw string `R"(…)"`.

     `extern "C"` deliberately does not count (detect.rs:266-268). Tests: `h_with_namespace_detected_as_cpp`, `h_with_template_detected_as_cpp`, `h_with_class_inheritance_detected_as_cpp`, `h_with_using_detected_as_cpp`, `h_enum_class_detected_as_cpp`, `h_constexpr_and_noexcept_detected_as_cpp`, `h_scope_resolution_detected_as_cpp`, `header_with_virtual_members_is_cpp`, `pure_c_header_stays_c`, `extern_c_header_stays_c`, `cpp_keyword_inside_comment_does_not_trigger`, `bitfield_struct_stays_c`, `include_next_of_c_header_stays_c` (detect.rs:747-1092).
  2. **Sibling heuristic.** A same-stem `.cpp/.cc/.cxx/.hpp/.hh/.hxx/.C` file in the same directory makes the header C++ (`header-heuristic:cpp`, detect.rs:235-255). Tests: `header_sibling_picks_cpp`, and `header_disambiguation_prefers_cpp_when_sibling_exists` (cgg/tests/detect.rs:88).
  3. Otherwise the header is **C** (`header-heuristic:c`). Test: `header_without_cpp_sibling_is_c`. Fixture f5: `plain.h` → `c header-heuristic:c`; `cppish.h` (`namespace n {…}`) → `cpp header-content:cpp`.
- **Obj-C headers are not recognised**: an `@interface` header is parsed as C. Fixture f11: `Foo.h` → `c header-heuristic:c`, `parse_status: error`. `has_cpp_content` has no Obj-C sniff, and the Obj-C plugin claims only `.m`/`.mm` (objc.rs:15-17).
- **CUDA `.cu`/`.cuh` are not claimed by any plugin.** Fixture f11: `k.cu` is skipped as `unknown-extension`. The C++ plugin's extensions are `.cc .cpp .cxx .C .hpp .hh .hxx` (cpp.rs:33-35).
- **Shebangs**: none. Fixture f5: an extensionless file starting `#!/usr/bin/tcc -run` → `unknown-extension`.
- **Content detection**: none beyond the `.h` rule.
- **Grammar**: one grammar, with no dialect switch.
- **Re-parse/fallback**: none; unlike C#, there is no `#if` blanking. A file with syntax errors is still extracted with `parse_status: "error"`. Fixture f1: `util.c` with a `;`-less `DEF_GETTER(x)` is `error` but its five functions are extracted.
- **Macro-swallowed headers.** A macro invocation with no `;` before a function makes the grammar fold the real header into the declarator. `swallowed_header` (c.rs:398-436) recovers the function name, and the swallowed `helper(void)` is not recorded as a call. Test: `a_macro_invocation_without_semicolon_does_not_swallow_the_next_function`. `LIST(DECL)` parsed as a `macro_type_specifier` return type is handled at c.rs:195-203.
- **Depth skip**: shared. A tree deeper than `MAX_TREE_DEPTH` (4,000) is skipped as `SkipReason::TooDeep` (cgg/src/lib.rs, `cgg_lang::exceeds_depth`). Test: `deep_nesting.rs` (a 3,000-term `sum.c` still extracts `h`, line 100-110).
- **Minified handling**: `max_line_bytes` is not overridden. `locals` has no C table, so the "no locals on lines over 2,000 bytes" rule has nothing to drop. The `--skip-minified` behaviour is generic and was not exercised for C (unverified).
- **System-header classification**: `is_platform_header` (cross_file.rs:3208+) knows C11/POSIX names and prefixes (`sys/`, `bits/`, `linux/`, `asm/`, `netinet/`, `mach/` …). It decides which `<…>` includes are never followed (§04).

## 02. Callables extracted

Every C callable is `DefVariant::FreeFunction` (kind `function`), with `qualified_name == simple_name`. There are no owners, scopes or file prefixes (c.rs:206-222, 237-249, 296-308). Two `static` functions of the same name in different files have identical QNs and are told apart only by file. Fixture f2: `dup_static` appears in both `a.c` and `b.c`.

Supported:

- **Function definition** `int add(int a, int b) { … }` → node with `has_body: true` (c.rs:170-223). Test `function_definitions_extracted`. Fixture f1.
- **K&R definition** `int knr(a, b) int a; int b; { … }` → node `knr`. Fixture f2.
- **Variadic** `int vfn(const char *fmt, ...)` → node. Fixture f2.
- **Pointer-returning and function-pointer-returning functions** `char *make(void)`, `static const char **two(int)` and `int (*get(void))(int)` are named by looking through the pointer, parenthesis and attribute declarators (`fn_name_from_declarator`, c.rs:262-285). A macro-built name `RET NAME_MACRO(f)(…)` → `f` (`ETHR_ATMC_FUNC__(xchg)` → `xchg`). Test `pointer_returning_functions_are_named_and_pointer_variables_are_not`. Fixture f2 `getter`.
- **Prototype** `int add(int a, int b);` and `char *proto(void);` → node with `has_body: false` (c.rs:225-254). Test `prototype_recorded`. It is then removed when a body with the same name and parameter list exists anywhere in the C family (`unify_declarations`, cpp.rs:1335+, called from cgg/src/lib.rs:534). Prototypes with no body in the tree survive as nodes. Fixture f8: `rs_add` and `asm_fn` in `ffi.h` are nodes.
- **Function-like macro** `#define SQUARE(x) ((x)*(x))` → `FreeFunction` with attribute `macro` (c.rs:287-312). It is also recorded as an `FnMacro` for expansion. Object-like `#define X 1` is not a callable. Fixture f1 `SQUARE ['macro']`.
- **Macro-generated functions**: `#define DEF_GETTER(n) int get_##n(void) {…}` invoked at file scope as `DEF_GETTER(x)` → node `get_x` with attribute `macro:DEF_GETTER` (c_macro.rs:779-895). See §08. Fixture f1 `get_x ['macro:DEF_GETTER']`; fixture f9 `made_a`, `made_b`.
- **GCC nested functions** `int outer(void) { int nested(int y) {…} return nested(2); }` → node `nested`, bound intra-file at high. Fixture f6.
- **`static inline` in a header** → node in the `.h`. Fixture f1 `inl`.
- **Functions under `#if 0`** are extracted like any other, because the grammar parses both branches. Fixture f9: `disabled` (inside `#if 0`) is a node, and `caller → disabled` is **high**.
- **`#ifdef`/`#else` alternatives** produce one node per branch. Fixture f9: `dbg` at lines 5 and 7. Fixture f2: `plat` at lines 8 and 10.

Not supported or partial:

- **No visibility**: `visibility` is always `""` (c.rs:215, 246, 305), so `static` vs external linkage is not recorded. Fixture f1: no `vis` column on any C node.
- **No attributes** on functions: `attributes: Vec::new()` (c.rs:216, 247). `__attribute__((constructor))`, `__attribute__((weak))`, `__declspec(dllexport)` and `__global__` are all dropped. Fixture f2: `init_ctor` has no attributes.
- **Prototypes whose parameter text differs from the body are not unified.** `unify_declarations` matches on `overload_key(signature_hint)`. Fixture f2: header `int apply(int (*f)(int), int x);` vs body `int apply(cb_t f, int x)` → **two `apply` nodes** (`a.h:2` and `a.c:13`).
- **A prototype whose bodies live in more than one other file is kept** (cpp.rs:1393-1409, the cuda-samples rule). Calls through the header bind to the bodyless prototype. Fixture f4: `go → solve@inc/api.h` (high); `p1/solve.c` and `p2/solve.c` get no edge.
- **Function-pointer variables, struct members and typedefs** (`int (*fp)(int);`, `struct ops { int (*run)(int); }`, `typedef int (*cb_t)(int);`) are not callables. Test: the `fp` assertion in `pointer_returning_functions_are_named_and_pointer_variables_are_not`.
- **No closures or lambdas** (C has none); Apple blocks `^{}` are not handled (no code path; unverified).
- **No `DefRecord::conditional`**: `#ifdef` branches are not marked as alternatives at extraction.

## 03. Call-site forms

`record_call` (c.rs:354-383) records only a `call_expression` whose `function` field is an `identifier` or a `field_expression`. It records `name` and `receiver_hint` (the argument text before `.`/`->`), and nothing else. There is no `arity`, `arrow`, `path_receiver`, `typed_receiver`, `paren_less` or `value_ref` flag.

Supported:

- **Bare call** `add(1, 2)` → `RefRecord{name:"add"}`. Test `call_expressions_captured`. Fixture f1.
- **Field / arrow call** `ptr->run()` and `o.run(3)` → `name:"run", receiver_hint:"ptr"`. Test `call_expressions_captured`. Fixture f1: `run` with `receiver_hint` `o` and `p`. A chained `n->next->fn()` → `name:"fn", receiver_hint:"n->next"` (fixture f2, external bucket).
- **Macro call** `SQUARE(3)` is a `call_expression` in tree-sitter-c and is recorded like a call. Test `macro_call_looks_like_call`. Fixture f1: `main → SQUARE` high.
- **Registrar arguments (value refs).** A function named by identifier in a call to a registrar verb becomes a `VALUE_REF_HINT` reference with `context` = the verb (c.rs:142-149 → `registrar::capture`, registrar.rs:424+). The verbs come from `rules::registrar_verbs_for("c")`, i.e. the `c-callbacks` registrars plus any user rule verbs (registrar.rs:431-436). The built-in verbs are `signal`, `atexit`, `at_quick_exit`, `on_exit`, `pthread_create`, `pthread_once`, `pthread_key_create`, `qsort` and `bsearch`. Test `functions_passed_to_runtime_registrars_are_value_refs` (`log_it(g)` is not captured). Fixture f5 with `--reference-edges`: `main → worker` and `main → bye` are edges with `via: reference`, at medium.
- **Swallowed header is not a call** (c.rs:112-118, 138-141). Test `a_macro_invocation_without_semicolon_does_not_swallow_the_next_function`.
- **Macro credit references.** A file-scope invocation of a function-like macro gets a `RefRecord{macro_call: true}` at its first byte (c_macro.rs:919-960).

Not supported:

- **Calls through a parenthesised or computed callee** are dropped (`_ => return`, c.rs:371). Examples: `(*fp)(6)`, `getter()()` (outer call), `table[i](x)`. Fixture f1: `(*fp)(6)` appears in no bucket. Fixture f2: only the inner `getter` of `getter()()` is recorded.
- **Function names passed as values to non-registrar calls** are not captured. Examples: `run_all(hook)`, struct initialisers `{ twice }`, and assignments `fp = twice`. The C plugin calls only `registrar::capture`, never `capture_value_refs`. Fixture f5: `run_all(hook)` gives no reference to `hook`. Fixture f1: `twice` (in an initializer and in `fp = twice`) has no incoming edge.
- **No arity recording**; C is not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583).
- **Calls written inside a `#define` body are not recorded**: `preproc_function_def` returns without walking its children (c.rs:127-133). Fixture f6: `#define LOG(...) do_log(__VA_ARGS__)` gives `k → LOG`, but no edge to `do_log`.
- **Explicit generics, operators, constructors and paren-less calls**: not applicable to C.
- **`_Generic` selections**: not handled specially (no code; unverified).

## 04. Imports & module resolution

Supported:

- **`#include "x.h"`** → `ImportRecord{kind:"include"}` (c.rs:319-332). Test `include_directive_captured`. In cross_file.rs:1789-1806 every definition of the included file goes into `direct_imports`. The closure is followed transitively to **depth 8** (`collect_include_defs`, cross_file.rs:3385+), memoised per translation unit by best remaining depth.
  - The path is resolved against the includer's directory, lexically normalised for `..` segments, then matched by exact path, then by suffix over the same-basename files. The lowest FileId wins, which keeps the result deterministic.
  - Tests:
    - `c_include_header_resolves` (cgg/tests/resolve.rs:444);
    - `c_parent_relative_include_resolves_to_the_named_header` (resolve.rs:2535), where `../common/util.h` beats a decoy `aaa/common/util.h`.
  - Fixture f1: `main → add`, `dupstr` and `inl` are all `cross-file:imports`.
- **`#include <x.h>`** → `ImportRecord{kind:"system-include"}` (c.rs:333-351). It is **followed like a quoted include** unless `is_platform_header` says it is a toolchain header (cross_file.rs:1807-1827). Test `angle_include_follows_an_in_tree_header_but_not_a_vendored_libc` (resolve.rs): `<helper_cuda.h>` binds and `<stdio.h>` never binds to a vendored `printf`. The system include also feeds framework detection (§10).
- **Several same-QN definitions reachable through the closure.** `c_include_copies` (cross_file.rs:7259-7360) keeps only those in the calling file or in a file the closure includes.
  - A closure holding only a prototype reaches the single body elsewhere, if there is exactly one (fixture f4: `shared_impl`, high).
  - Bodies all in one outside file are treated as preprocessor alternatives and bound **once, to the last** (fixture f2: `use → plat@a.c:10` **high**).
  - Several definitions in one in-closure file bind once, to the last (fixture f10: `f → ASSERT@dbg.h:4` **high**, while `t/t.h`'s unrelated `ASSERT` is excluded).
- **Cross-language family lookup.** A miss in `"c"` retries the QN under `cpp` and `objc` (`lookup_with_reexports`, cross_file.rs:7366+), so a C++ file including a C header reaches the `.c` body.
- **Step 1d global fallback.** When the file's include closure contributed at least one name (`has_imports`), an unresolved bare name with at most 3 same-language candidates that is not in `stdlib/c.txt` binds by name at **medium** (cross_file.rs:6015-6045). Fixture f2: `use → noproto`, `knr`, `vfn` and `getter` are medium even though `a.h` does not declare them.
- **Object-like macro receivers** (`#define THEKERNEL Kernel::instance`): type_hints.rs:640 applies to `"c" | "cpp"`, but only the C++ plugin fills `macro_aliases` (cpp.rs:677). For C the branch is inert.

Not supported:

- **A file whose includes resolve to nothing gets no global fallback.** Fixture f6: `noinc.c` (no includes) calling `hidden()`/`counter()`, which are defined only in `lib.c`, gives `candidates-in-other-files`. `shadow.c` with `#include "nothing.h"` (missing) gives the same.
- **A macro-generated function is not reachable without a prototype.** Fixture f1: `main.c` calling `get_x()` (generated in `util.c`, not declared in `util.h`) gives `no-candidate-in-file`. `macro:` definitions are kept out of by-name fallback.
- **No `FileFacts::packages`, manifests, `heads.rs` external-module refusal or `not-in-imported-module`.** heads.rs:168 and :194 add C-family `cpp_scopes` from `field_types`, `type_aliases` and `::` QNs, none of which the C plugin produces. The `external` bucket for C is only the post-hoc classification "name defined nowhere in the tree and not stdlib" (cgg-core/src/external.rs:1-35). Fixture f1: `fp` and `run` are `name_screen_applied: external`.
- **Re-exports and aliases**: not applicable to C.
- **`#include` of a computed path** (`#include MACRO`) is not recorded: the path node is neither `string_literal` nor `system_lib_string`.

## 05. Type inference & receiver typing

None for C:

- The plugin records no `local_types`, `field_types`, `type_aliases`, `scoped_types` or parameter types. `macro_aliases` is filled only by the C++ plugin (cpp.rs:677).
- The only `local_types`/`locals` a C file can receive come from macro expansions (c_macro.rs:868-891). The C fragment extractor produces no `local_types` (c.rs:68-84), and `locals::collect` has no C table (locals.rs:380-393).
- Struct-member chains (`kernel->streams->run()`) are typed only for `facts.language == "cpp"` (type_hints.rs:578).
- No casts (`CAST_INTRINSICS` lists only cpp and scala, stdlib.rs:35-46) and no constructions.
- No `declared_type` reading, no closure typing, no implicit or weak receivers.

Consequence: field and arrow calls go through name-based lookup only. Fixture f4: `p->run(1)` and `o.run(2)`, where `run` is a struct function-pointer field, both bind to the unrelated free function `run@impl.c` at **medium**. These are false edges.

## 06. Inheritance, dispatch & overloading

Not applicable to C, which has no classes, inheritance or overloading. Specifically:

- Not in `INHERITANCE_LANGS` (dispatch.rs:110-123), so there is no `inheritance_fanout`.
- No `trait_impl_target`, so `--dynamic-dispatch` adds nothing.
- Not in `OVERLOAD_ARITY_LANGS` (type_hints.rs:1583).
- No `member_of_calling_class`, `overrides_below`, `super` or extension methods.
- **Function-pointer "vtables"** (struct of function pointers, the C OO idiom) are not modelled. There is no dispatch from `ops->run()` to the functions stored in `ops`. Fixture f1: `struct ops o = { twice }; o.run(3)` gives no edge to `twice`. Fixture f4: `run` binds by name to an unrelated function. The `c-runtime` gap text (rules.rs:7945+) names this pattern.

## 07. Scoping & shadowing

- **No `FileFacts::locals` for C** (locals.rs:380-393). As a result, no `shadowed-by-local` refusal is possible. Fixture f7:
  - `int h(int (*f)(int)) { return f(1); }` with `f` declared in the included `lib.h` → `h → f@lib.c` **high**;
  - `int k(void) { int (*counter)(void) = 0; return counter(); }` → `k → counter@lib.c` **high**.

  Both are false high edges.
- **No lexical narrowing or positional shadowing** for C.
- **Same-file duplicate definitions are refused intra-file.** Fixture f9: `#ifdef DEBUG static int dbg… #else static int dbg… #endif` with `caller()` in the same file gives `dbg` unresolved, `ambiguous-in-file` (`file_local: 2`). The cross-file path instead binds the last alternative at high (§04). The two paths are inconsistent.
- **`static` file scope is not modelled**, because there is no visibility. Step 1d can bind a bare call in `b.c` to a `static` function in `a.c` (unverified for a `static` target specifically). Fixture f2 shows `noproto` (non-static) bound from `b.c` without a prototype in scope.
- **`#if 0` code is in scope** (fixture f9, §02).
- **Conditional definitions**: there is no `DefRecord::conditional` marking. Preprocessor alternatives are handled only by `c_include_copies` in the cross-file path.

## 08. Macros, preprocessor & codegen

- **Function-like `#define` as a callable** with attribute `macro` (c.rs:287-312). Calls to it bind like calls to functions. Fixture f1: `main → SQUARE` is high. Fixture f6: `k → LOG` is high.
- **Function-macro expansion** (`c_macro.rs`, run after all C/C++ files are extracted and before prototype unification; cgg/src/lib.rs:524-533):
  - Only **item-position** invocations are expanded: file scope, outside `NOT_ITEMS` kinds (c_macro.rs:94-123, `scan_invocations` 127-151).
  - The macro used is the last same-named `#define` earlier in the file, else the first one found in the `#include "…"` closure (`INCLUDE_DEPTH` = 8, c_macro.rs:36, 744-756).
  - Expansion is **one level**: `#` stringize, `##` paste, `__VA_ARGS__` and the GNU comma (`substitute`, 483+). Expansions over 64 KiB are not parsed (`MAX_EXPANSION_BYTES`, :34).
  - Only `function_definition`s that parse with no error are kept (`collect_clean_functions`, 898-916). They are tagged `macro:<name>`, given slot spans inside the invocation, and calls in their bodies are re-attributed to them (779-895).
  - Tests: `pastes_and_stringizes`, `variadic_arguments_and_gnu_comma`, `header_macro_generates_functions_in_the_including_file`, `macros_that_define_no_function_or_are_not_visible_generate_nothing`, `arguments_split_on_parenthesis_depth_only`.
  - Fixture f9: `FN_DEF(made_a, 1)` and `FN_DEF(made_b, made_a())` → nodes `made_a` and `made_b`, with `made_b → made_a` high and `use2 → made_b` high.
- **Credit and name mentions** (`credit`, c_macro.rs:919-990):
  - A file-scope invocation gets a `macro_call` reference to the macro.
  - An invocation that generated nothing records its identifier arguments as `name_mentions` (`via: "macro-arg"`). An argument that is itself a function-like macro is credited instead.
  - Fixture f9: `LIST(DECLARE)` credits both `LIST` and `DECLARE`. Fixture f12 (with `;`): `REGISTER(handler);` roots `REGISTER` (`toplevel:invocation`), and `handler` carries the `name-collides-with-screened-site` evidence.
- **No nested expansion.** Fixture f9: `#define LIST(X) X(alpha) X(beta)` with `LIST(DECLARE)` generates nothing, and `beta` is never declared.
- **Invocations inside function bodies are never expanded** (by design, c_macro.rs:9-11). Fixture f12: `REGISTER2(inside)` inside `main` → `inside` is not credited and is reported dead.
- **A `;`-less invocation before a `static` function goes uncredited.** Fixture f12, first variant: `REGISTER(handler)` followed by `static void inside(void)`. The macro is not credited (`REGISTER` reported dead), and `inside`'s span and `signature_hint` start at the invocation (`"REGISTER(handler) static void inside(void)"`, `start_line` 3).
- **Calls inside a `#define` body are not recorded** (§03; fixture f6: `do_log` is not reached via `LOG`).
- **Object-like macros are not followed** (§04/§05). Fixture f6: `#define ALLOC my_alloc` then `ALLOC(3)` → `ALLOC` lands in the `external` bucket, and `my_alloc` is not reached.
- **`#if`/`#ifdef` are not evaluated.** Both branches are parsed (fixture f9: `#if 0` content is live). Alternatives are reconciled only by `c_include_copies` (§04).
- **Templates, quote/unquote**: not applicable.

## 09. FFI & cross-language links

- **asm ↔ C** (`ffi.rs` Pass A, 36-186):
  - **asm → C.** An asm `call c_helper` links to a C-family callable of that name. A single leading `_` is stripped only when no C function carries the name as written. The edge is `Via::Ffi("asm-c")` at medium.
  - **C → asm.** A C reference whose name, or the name with leading `_`s trimmed, is an asm label gives `Via::Ffi("c-asm")` at medium. Value and string references are skipped (ffi.rs:128-135).
  - Tests: `a_value_reference_to_an_asm_label_is_not_an_ffi_call`, `an_asm_call_strips_at_most_one_underscore_and_only_when_needed`.
  - Fixture f8: `asm_fn@a.S → c_helper` (asm-c, medium), and `main → asm_fn@a.S` (c-asm, medium) alongside `main → asm_fn@ffi.h` (prototype, high).
- **Rust/C# exports → C callers** (Pass B, ffi.rs:188-262). A `c-abi` export (`#[no_mangle]`, `extern "C"`, `[UnmanagedCallersOnly]`, `dllexport`; ffi.rs:660-665) accepts C-family callers only (`allowed_source_languages`, ffi.rs:698).
  - The edge goes from a **same-named C callable** (in practice the C prototype node), not from the call site, at `site_line: 0`, medium.
  - Fixture f8: `rs_add@ffi.h → x::rs_add@src/lib.rs` (`ffi`/`c-abi`, medium). `main → rs_add@ffi.h` is high.
- **C as an FFI export side: none.** The C plugin records no attributes, so `__declspec(dllexport)` and `__attribute__((visibility("default")))` are never seen, and `classify_ffi` cannot fire on a C callable.
- **JNI (`Java_pkg_Cls_m`), PyO3, napi, P/Invoke, wasm-bindgen into C**: no C-side matching. Pass B only links exports that carry a family attribute. A Java `native` method has no family-specific C name matching (no code path found; unverified beyond the grep of ffi.rs).
- **Descriptor links** (protobuf rpc → host language): not applicable to C (no C target in descriptor.rs; unverified beyond grep).
- **Markup code-behind**: not applicable.

## 10. Frameworks & entry points

Rules whose `language` is `"c"` (rules.rs). `RuleSpec.language` is a single string, and matching is exact `f.language == rule.language` (frameworks/mod.rs:241, 596, 648), so `cpp` rules never apply to C files.

| rule id | kind | trigger | effect |
| --- | --- | --- | --- |
| `cuda` (rules.rs:1399) | Lifecycle | attribute `__global__` (marker-only rule) | Mints a node per kernel. **Can never fire for C**: the C plugin records no attributes, and `.cu` is not a recognised extension (fixture f11). The live CUDA path is the `cpp` rule (rules.rs:1378) via `cuda_qualifiers` (cpp.rs:2713). |
| `c-runtime` (rules.rs:7945) | Lifecycle | detect on system-include paths `signal.h`, `pthread.h`, `stdlib.h`, `setjmp.h`, `stdio.h`, `unistd.h`, `sys/signal.h` | `node: false`; no attributes, registrars or methods. It exists to **report a coverage gap** (`gap_for`): sigaction handlers, constructor/destructor attributes, ISR vector tables, longjmp targets, and function pointers in variables or struct fields. Fixture f1/f3: listed under "seen, no rules". It can fire only because `system-include` records exist (c.rs:333-351; CHANGELOG line 5043). |
| `c-callbacks` (rules.rs:7976) | Lifecycle | `detect_calls` and `registrars` = `signal`, `atexit`, `at_quick_exit`, `on_exit`, `pthread_create`, `pthread_once`, `pthread_key_create`, `qsort`, `bsearch` | `node: false`. Root-marks the in-tree function named by identifier in the arguments. Fixture f1/f3: `on_sig` (via `signal`) and `cmp` (via `qsort`) are roots with `framework:rule` "invoked by framework rule `c-callbacks:registrar`". The audit reports `recognised c-callbacks (lifecycle, 2 entries)`. |

- **member_sources, signature_methods, container_attributes, markup_kinds, visibility_entries**: none for C (no `"c"` entry; grep of rules.rs helper tables 11666-11870).
- **Registrar verbs**: the nine `c-callbacks` verbs above, plus any user-supplied rule verbs (`ExtractCtx`).
- **Program entry roots** (dead code, not framework nodes): `main` (`UNIVERSAL_ENTRY`), plus `_start`, `WinMain` and `DllMain` (`LANG_ENTRY`, deadcode/roots.rs:27).
- Not covered (as stated by the `c-runtime` gap): `sigaction`, `__attribute__((constructor/destructor))`, ISRs, `dlsym` targets, plugin tables, GLib/GObject signal connects, Linux kernel `module_init`/`EXPORT_SYMBOL`, libuv/libevent callbacks. None of these have rules; for the last four this comes from a grep of rules.rs ids with `language: "c"` (only three exist).

## 11. Test harness entries

- **No test-kind framework rule for C.** There is no Unity (C), cmocka, Check, CUnit, Criterion, greatest or munit rule. A grep of rules.rs for these names finds only C#'s Unity engine rule (3931) and CMake's CTest gap prose. No `<framework-entry>::test::*` node is ever minted for C.
- **Name-based test roots** (dead code only): `is_test_name` default arm, `name.starts_with("test_") || name.starts_with("Test")` (deadcode/roots.rs:975-990) → `builtin:test-name`, `RootKind::TestEntry`. Fixture f3 roots: `test_something`, `test_one`, `TestTwo`. This is a bare prefix match, so `Testament()` or `test_harness_util` also count.
- **Test-file classification** (`classify_test_file`, testfile.rs:111-117) for `"c" | "cpp"`:
  - names ending `_test.c`, `_test.cc` or `_test.cpp`;
  - names starting `test_`;
  - names containing `_unittest.`;
  - the generic test-directory rule (testfile.rs:126-133).

  Fixture f3: `helper_t` in `test_b.c` is withheld from findings unless `--show-test-findings` is passed.
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** is python/php/ruby only (rules.rs:11806): not applicable.
- **No `TestRole`** is set by the C plugin (`test_defs` is not declared).

## 12. Dead-code analysis

- **Capability class: `degraded`.** C declares no `visibility` and no `attributes` (c.rs:28-33), and observation can upgrade a signal only if a C callable carries one (caps.rs:112-200). Function-like macros carry `macro`, so a tree with any `#define F(x)` observes `attrs: yes` (fixture f3 table: `vis no, attrs yes, refs yes, disp no`). Visibility never appears, so `signals_complete()` is false and **every C finding is capped at medium**. Fixture f4 capabilities: `class degraded, max_confidence medium`.
- **Root kinds that apply to C**:
  - `builtin:main` for `main`, `_start`, `WinMain`, `DllMain` (roots.rs:739-757);
  - `framework:rule` from `c-callbacks`;
  - `builtin:test-name` (§11);
  - `toplevel:invocation` for a call outside any function (roots.rs:552-614);
  - `toplevel:macro-invocation`, for a C-family `macro` callable whose name has an unresolved site with no enclosing callable (roots.rs:616-660; fixture f3: `DEF_GETTER` rooted, "macro invoked outside any function at ./util.c:8");
  - `user:roots`.
- **No `ffi:export`, `lang:export`, lifecycle or conventional roots** for C. `LIFECYCLE` has no `c` row, `CONVENTIONAL_METHODS` are OO names, and C records no exports.
- **Evidence and caps seen on C findings**:
  - `language-lacks-dispatch-model`, `language-lacks-visibility` and `language-lacks-value-references` (when no reference edge landed);
  - `macro-generated` (fixture f3: `get_x`);
  - `name-matches-unresolved-site`;
  - `name-collides-with-screened-site` (fixture f12: `handler`);
  - `incoming-only-from-dead-code` (D002).
- **Unreachable statements** after `return/break/continue/goto/throw` are extracted under `--dead-code` (`C_LIKE`, cfg.rs:122-133; c.rs:58-61). `switch` cases are never flagged (test `c_switch_cases_are_never_flagged`, cfg.rs:284). **No consumer found:**
  - a grep for readers of `FileFacts::unreachable` in cgg-resolve and cgg finds none;
  - fixture f4 (`y = 3;` after `return`) gives no "unreachable"/"after-return" text in the graph JSON, the audit or the dead-code JSON.
- **False dead findings documented by fixture**:
  - `__attribute__((constructor))` functions (f3 `init_ctor`);
  - functions stored in struct initialisers or function-pointer variables (f3 `twice`);
  - functions called only from a `#define` body (f6 `do_log`, implied by the missing edge);
  - functions named in an in-body macro invocation (f12 `inside`).
- **Static vs external**: not distinguished. An external-linkage library function with no in-tree caller is reported just like a `static` one (fixture f3 `unused_fn`, `noproto`).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/c.txt`**: 278 lines, 2,319 bytes, loaded at stdlib.rs:232. It holds libc/POSIX names: `abort`, `abs`, `accept`, `access`, … `atexit`, `bsearch`, `calloc`, … `printf`, `qsort`, `signal`, `strdup`, … `pthread_create`.
- **Classification**: an unresolved call whose bare name is in the table goes to the `stdlib` bucket (`name_screen_applied: "stdlib"`), unless the project defines that name, in which case it is a real unresolved site (cgg-core/src/external.rs:28-35). Fixture f1: `signal`, `qsort` and `printf` are stdlib; `strdup` is stdlib in `util.c`.
- **Step 1d skips stdlib names** for the by-name fallback (cross_file.rs:6022-6024).
- **`--include-stdlib` / `--include-external`** mint deduplicated exit nodes `<stdlib>::name` / `<external>::name` (synthetic, language `c`), with `Via::Stdlib`/`Via::External` edges at **low**. Fixture f5: `<stdlib>::pthread_create`, `<stdlib>::atexit`, `<stdlib>::printf` and `<external>::deflate` (from `<zlib.h>`).
- **`is_platform_header`** (cross_file.rs:3208+) is a second builtin table: C11, POSIX and OS header names and prefixes that are never followed as includes.
- **None for C**: `CAST_INTRINSICS`, `CONSTRUCTING_METHODS`, `MAYBE_TYPE_WRAPPERS`/`UNION_TYPE_CONSTRUCTORS`/`NULL_TYPE_MEMBERS`, `rust_types`-like wrapper tables, `RUST_UNTYPED_ADAPTER_METHODS`-like adapters, and implicit packages.
- **Platform headers vs stdlib table**: a vendored libc in the tree is not followed for `<stdio.h>` (test `angle_include_follows_an_in_tree_header_but_not_a_vendored_libc`).

## 14. Confidence & certainty

Paths observed to yield **high** for C:

- Intra-file single candidate: `walk → dup_static`, `caller → disabled`, `use2 → made_b` (fixtures f2, f9).
- Include closure, single QN: `main → add`, `main → SQUARE`, `main → inl` (fixture f1).
- `c_include_copies` choosing a prototype's single outside body: `go → shared_impl` (f4).
- Include closure landing on a **bodyless prototype** (f4 `go → solve@api.h`; f8 `main → rs_add@ffi.h`).
- Same-file preprocessor alternatives, bound once to the last: f2 `use → plat@a.c:10`; f10 `f → ASSERT@dbg.h:4`. **This contradicts CHANGELOG.md:176-178** ("Conditional alternatives in one header … now bind both at `medium` instead of one at `high`"). The code (cross_file.rs:7316-7360, "bound once — the last") and both fixtures bind one at high.
- **False highs**: a function-pointer parameter or local named like an included function (§07, fixture f7).

What yields **medium**:

- Step 1d by-name fallback (f2 `noproto`, `knr`, `vfn`, `getter`).
- Field/arrow calls bound by name (f4 `p->run`, `o.run`).
- All FFI edges (`asm-c`, `c-asm`, `c-abi`).
- `--reference-edges` registrar references (f5).

What yields **low**: exit-node edges (`--include-stdlib`/`--include-external`), and framework-entry edges when such a node exists (none are minted for C).

**Mermaid `?` marker**: an arrow with no high site gets a leading `?`. Fixture f2 mermaid: `N12 -->|?| N1` (`use → noproto`), while `N12 --> N6` (`use → plat`) is unmarked.

## 15. Dynamic use & reflection

- **No C entry in `dynuse::REFLECTIVE`** (dynuse.rs:25-56: python, ruby, java, csharp, javascript, typescript, php, go). Consequences:
  - `dlsym(h, "name")`, `GetProcAddress(h, "name")` and string-keyed dispatch tables neither cap findings nor produce edges;
  - the C plugin does not declare `dyn_uses` and does not call `dynuse::extract`.
- **`DYN_MODULE_LOADERS`** (`require`, `importScripts`, `import_module`; facts.rs:165): none apply to C. `dlopen` is not treated as a module load.
- **String references**: the registrar capture may emit `STRING_REF_HINT` records for string arguments of registrar verbs. These never become edges (cross_file.rs:2246-2248) and are skipped by the C→asm pass (ffi.rs:132-133).

## 16. Known gaps & limitations

From code comments, gap text and CHANGELOG:

- The `c-runtime` `gap_for` text (rules.rs:7945+) lists what is invisible:
  - sigaction handlers;
  - `__attribute__((constructor))` / `__attribute__((destructor))`;
  - ISRs wired through vector tables or linker scripts;
  - longjmp targets;
  - callbacks passed through a variable or cast;
  - function pointers stored in structs ("the vtable pattern that carries most C plugin and driver architectures").
- The `cuda` rule for `c` (rules.rs:1399) is unreachable: no attributes are extracted, and `.cu` is not an extension of any plugin (fixture f11).
- README (README.md:493) row "C | `#include` transitive (depth 8) | — | Macros as callables", where `—` means no type inference.
- CHANGELOG.md:2175-2179 (Known limitations): a `.h` without a C++ sibling or C++ content is parsed as C, so class declarations in it are invisible to the C++ resolver (188 of 323 headers on one firmware repo).
- `c_macro.rs` header (1-25) is limited by design: one level, item position only, clean function definitions only.

Found while cataloguing (fixtures):

1. **False high edges through function-pointer parameters and locals**: no `locals` table for C (f7).
2. **False medium edges for struct function-pointer member calls**: `p->run()` binds to any free `run` (f4). There is no receiver typing for C at all.
3. **Preprocessor alternatives are handled inconsistently**. Intra-file they are `ambiguous-in-file` (f9 `dbg`); cross-file they bind one at **high** (f2, f10), contrary to CHANGELOG.md:176-178. `#if 0` code is extracted and bound at high (f9).
4. **No visibility or attributes**. Dead code is `degraded` (medium max), and `static` vs external linkage is indistinguishable.
5. **Call shapes dropped**: `(*fp)()`, `getter()()`, `tbl[i]()`. Calls inside `#define` bodies (f1, f2, f6).
6. **Value references only for the nine registrar verbs**. `run_all(hook)`, `struct ops o = { twice }` and `fp = twice` give no reference (f1, f5).
7. **Prototype/body parameter text mismatch** (`cb_t f` vs `int (*f)(int)`) leaves a duplicate prototype node (f2 `apply`).
8. **A prototype with bodies in several other files absorbs the call** at high (f4 `solve`).
9. **Cross-file calls need the include closure to contribute something**. A file with no resolvable include gets no fallback (f6), and macro-generated functions are unreachable without a prototype (f1 `get_x`).
10. **Object-like macro aliases** (`#define ALLOC my_alloc`) are not followed for C (f6). `macro_aliases` is C++-only.
11. **A `;`-less macro invocation before a `static` function** is not credited, and the function span starts at the invocation (f12).
12. **Obj-C `@interface` headers are parsed as C**, with parse errors (f11).
13. **Unreachable-statement extraction has no consumer** (f4).
14. **No C test harness rules**; test detection is a bare `test_`/`Test` name prefix.
15. **No dynuse** (`dlsym`/`GetProcAddress`).
16. No `#[ignore]`d tests reference C. A grep of `crates/*/tests` and `c.rs`/`c_macro.rs` for `#[ignore]` near C tests found none (unverified beyond that grep).

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01. Detection & parsing | full | `.c`/`.h` with content and sibling `.h` disambiguation; no `.cu`, no Obj-C header sniff, no shebang |
| 02. Callables extracted | partial | functions, K&R, prototypes, fn-macros, macro-generated fns; no visibility/attributes; flat QNs; mismatched prototypes duplicate |
| 03. Call-site forms | partial | bare, field/arrow, macro calls, registrar value refs; drops `(*fp)()`, `f()()`, non-registrar value refs, calls in `#define` bodies |
| 04. Imports & module resolution | partial | `#include` "…" and <…> closure to depth 8 with copy narrowing; no packages/heads refusal; fallback needs a resolvable include |
| 05. Type inference & receiver typing | none | no locals, fields, params or casts; member calls bind by name |
| 06. Inheritance, dispatch & overloading | n/a | language has none; function-pointer vtables not modelled |
| 07. Scoping & shadowing | none | no locals table, so fn-pointer params/locals yield false high edges; same-file alternatives refused |
| 08. Macros, preprocessor & codegen | partial | one-level item-position expansion, `macro:` tagging, name_mentions; no nested expansion, no `#if` evaluation, no object-like aliases |
| 09. FFI & cross-language links | partial | asm↔C both ways, Rust/C# c-abi exports ← C prototypes; C never an export side |
| 10. Frameworks & entry points | minimal | `c-callbacks` (9 verbs), `c-runtime` gap-only, `cuda` (dead for C) |
| 11. Test harness entries | minimal | no harness rules; `test_`/`Test` name prefix and test-file names only |
| 12. Dead-code analysis | partial | degraded class (medium max); main/_start/WinMain/DllMain, macro-invocation roots; unreachable regions not consumed |
| 13. Builtins & stdlib tables | partial | `stdlib/c.txt` (278 names) and platform-header table; no cast/type tables |
| 14. Confidence & certainty | partial | high for include-closure singles, but false highs via fn-pointer shadowing and alternatives |
| 15. Dynamic use & reflection | none | no dynuse entry (`dlsym`, `GetProcAddress` unseen) |
| 16. Known gaps & limitations | n/a | 16 items listed; CHANGELOG/code disagreement on conditional alternatives |
