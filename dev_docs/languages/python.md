# python

The Python plugin (`crates/cgg-lang/src/plugins/python.rs`, 1,978 lines, `PythonPlugin`, id `"python"`) does one recursive AST pass over a `tree-sitter-python` tree (workspace dependency `tree-sitter-python = "0.25"`, `Cargo.toml:83`; `ts_language` at `python.rs:58-60`; no vendored grammar). It extracts definitions (`def`, `async def`, methods, named lambdas), call sites (`call` nodes whose callee is an `identifier` or `attribute`), imports (`import` / `from … import`), class records with bases, class-level field declarations, instance-field types, an `__all__` export list, dead-code signals and test roles. It is registered at `crates/cgg-lang/src/plugins.rs:125`. It declares the signals `attributes, dyn_uses, exports, impls, test_defs, unreachable, value_refs, visibility` (`python.rs:44-56`). Python has no plugin-specific "resolver kind" switch beyond the shared pipeline. Language-specific branches live in `cgg-resolve` (`cross_file.rs`, `modules.rs`, `heads.rs`, `intra_file.rs`, `type_hints.rs`, `dispatch.rs`, `ffi.rs`, deadcode) and are cited section by section below. Jupyter notebooks (`.ipynb`) go through the same plugin after their code cells are extracted.

Fixtures for every "fixture" claim below are under `<scratch>/parity/python/f1` … `f20`. They were run with `target/release/cgg` 0.11.0 (`-t json -o out.json`, then parsed by `show.py` in the same directory).

## 01. Detection & parsing

Supported:

- **Extensions** `.py`, `.pyi`, `.ipynb` (`python.rs:38-40`). Fixtures f9/f16: `stub.pyi` is detected as `extension:.pyi`, and `nb.ipynb` yields callable `nb.nbf`.
- **Shebangs** `python3`, `python`, `python2` (`python.rs:41-43`). Test `crates/cgg-lang/src/detect.rs:708 python_shebang_beats_extension`. Fixture f9: an extensionless `tool` with `#!/usr/bin/env python3` yields `tool.shebang_fn`.
- **Notebooks**: `crates/cgg/src/lib.rs:405-423` routes `.ipynb` through `cgg_lang::notebook::extract_python_source`. That function keeps only `cell_type == "code"` cells, joins them with blank lines (`notebook.rs:22-60`), and drops lines starting with `%`, `!` or `?` (`notebook.rs:70-85`). Malformed JSON is skipped as `ParseError("ipynb: malformed notebook JSON")` (`lib.rs:414-418`). Tests: `notebook.rs extracts_code_cells_and_skips_markdown`, `strips_ipython_magics`.
- **Grammar**: a single grammar for every Python file. There is no per-file grammar switch (`ts_language_for` is not overridden).
- **Depth guard**: a file whose tree is deeper than `MAX_TREE_DEPTH = 4000` is skipped as `SkipReason::TooDeep` (`lib.rs:155, 451-456`). This applies to every language.
- **Minified guard**: `FileFacts::locals` is not collected when any line exceeds `MINIFIED_LINE_BYTES = 2000` (`lib.rs:159, 476-486`). `CertainCtx::minified_caller` (average line over 1,000 bytes) keeps the edges of such a file below `high` (`cross_file.rs:4545-4550`).
- **Parse errors**: a tree with `has_error()` is still extracted, with `parse_status: "error"` (`lib.rs:457-461`).
- **Module naming**: the dotted path is built from the parent directories that contain `__init__.py` (`module_name`, `python.rs:139-159`). A package's `__init__.py` gives qualified names such as `pkg.__init__.f`, but its importable name (`FileFacts::packages`) is `pkg` (`python.rs:72-82`). Test: `packages_hold_the_importable_module_name`.

Not supported / partial:

- **No per-plugin long-line cap**: `max_line_bytes` is not set for Python (only Kivy sets it; README "Limitations").
- **Non-Python notebook kernels**: R or Julia kernels are parsed as Python and produce parse errors (`notebook.rs:11-15`).
- **Cell magics**: only the `%%magic` line is dropped. The body of a `%%bash` cell stays and is parsed as Python (`notebook.rs:70-85`). Unverified by fixture.
- **Inline shell assignment**: `x = !ls` is kept, because the line does not start with `!` (`notebook.rs:74-77`).
- **Implicit namespace packages (PEP 420)**: a directory without `__init__.py` contributes no module segment, so the qualified name falls back to the file stem (`python.rs:145-156`).

## 02. Callables extracted

Supported (variant decided in `record_function`, `python.rs:491-550`):

- **Free function** `def f(): …` becomes `FreeFunction`, qualified name `module.f`. Test `free_functions`.
- **Async function** `async def coro(): …` becomes `AsyncFunction`. Test `async_function`.
- **Method** `class C: def m(self): …` becomes `InherentMethod`, qualified name `module.C.m`. It is a method only when the immediate scope is a class (`scope_is_class`, `python.rs:504-511`). Test `class_methods_get_class_in_name`, `function_nested_in_a_method_is_not_a_method`.
- **Constructor** `def __init__(self)` inside a class becomes `Constructor`. **Destructor** `__del__` becomes `Destructor` (`python.rs:512-515`). Test `init_is_constructor`. Fixture f18: `__del__` has kind `destructor`.
- **Static method and class method** (`@staticmethod`, `@classmethod`) become `StaticMethod` / `ClassMethod`, matched by substring on the decorator text (`python.rs:516-519`). Test `static_and_class_methods_are_variants`.
- **Property** `@property def p(self)` becomes `Property` (`python.rs:520-521`). Fixture f2: `shapes.Square.side` has kind `property`.
- **Named lambda** `square = lambda v: v * v` becomes `NamedLambda`, kind `closure`, qualified name `module.square` (`named_lambda`, `python.rs:726-761`). Test `named_lambda_is_callable`. Fixture f1: `pkg.core.square`.
- **Nested function** `def outer(): def inner(): …` gets qualified name `module.outer.inner` (scope push at `python.rs:246-262`). Test `nested_function_qualified_name`. Fixture f1: `pkg.core.outer.inner`.
- **Nested class** `class X: class Inner: def im(self)` gives `module.X.Inner.im`. Fixture f17.
- **Class nested in a function**: the class is recorded. If its name occurs more than once in the file, it is never used as a receiver type (`python.rs:95-117`). Test `a_duplicated_function_local_class_is_never_a_receiver_type`.
- **Overloads** (`@typing.overload` stubs plus the implementation): each `def` is a separate callable with the same qualified name, and the stubs carry attribute `@overload`. Fixture f2: three `base.Proc.f` nodes.
- **Abstract members** `@abstractmethod` are ordinary callables carrying the attribute. Members of a class with base `Protocol`/`ABC`/`ABCMeta` are callables as well. Both are treated as declarations at resolve time (section 06).
- **Conditional definitions** (`if …: def f` / `else: def f`): both are extracted as separate callables. Fixture f2: two `misc.cond` nodes.
- **Decorated functions**: decorators are collected from `decorated_definition` into `DefRecord::attributes`, verbatim with `@` (`collect_decorators`, `python.rs:949-978`). Fixture f2: `wrapper` carries `['@functools.wraps(fn)']`.
- **`.pyi` stub definitions** are ordinary callables (`def f(x: int) -> int: ...`). Fixture f9/f16.
- **Owner and qualified-name formation**: `scope.join(".")`, where scope starts as `[module]` (`qualified_name`, `python.rs:937-941`).
- **Every callable** gets `signature_hint` (the `def` line, via `extract_signature`), `vis` (section 12), `test_role` (section 11), `attributes` (decorators) and `base_types` (enclosing class bases) (`python.rs:534-549`).

Not supported / partial:

- **Anonymous lambdas** (`sorted(key=lambda x: …)`, `cb=lambda: …`) are not callables. Calls inside them belong to the enclosing def.
- **Lambda bound to an attribute or tuple target** (`self.f = lambda: …`, `a, b = lambda: 1, …`): `named_lambda` requires `left` to be an `identifier` (`python.rs:733-735`).
- **Class-body method aliases** `run = _run` create no callable. The right-hand side is only a value ref with no enclosing callable (`value-ref-no-enclosing`, fixture f2). `Aliased().run()` binds nothing.
- **Property setter and deleter** `@side.setter` do not contain the substring `property`, so they become `InherentMethod` with the same name as the getter. Fixture f2: `shapes.Square.side` appears once as `property` and once as `method`.
- **Substring decorator matching**: any decorator text containing `staticmethod`/`classmethod`/`property` changes the variant, for example `@cached_property` → `Property` (`python.rs:516-521`). A hypothetical `@not_a_staticmethod_helper` would also match. Unverified by fixture.
- **`__new__`** is a plain `InherentMethod`, not a constructor (fixture f18). `dispatch.rs:397` excludes `__init__`/`__new__`/`__del__` from overriding.
- **Async methods** get variant `AsyncFunction`, and the graph kind of a method is then `function` (fixture f18: `a.A.am function`). This affects the dead-code external-contract rule (section 12).
- **Module top level**: no `<module>` callable exists. Calls at module or class top level have no enclosing callable and become dead-code `toplevel:invocation` roots (fixture f7: `app.on_exit`).
- **`exec`/`eval`-generated, `type()`-built or metaclass-generated methods** are not extracted. No code path handles them (grep: no `type(` handling in `python.rs`).

## 03. Call-site forms

Supported (`ref_from_call`, `python.rs:794-849`; walker `python.rs:279-325`):

- **Bare call** `f()` is recorded with name `f` and an empty receiver. Fixture f1.
- **Method or attribute call** `a.b.c()` is recorded with name `c` and receiver `a.b`. An attribute chain split across lines is rebuilt (`receiver_text`, `python.rs:782-792`). Test `a_wrapped_attribute_chain_receiver_matches_the_one_line_form`.
- **Constructor call** `Foo(…)` is recorded as a bare call to `Foo`. Cross-file step 6 maps it to `Foo.__init__` (`constructor_names`, `cross_file.rs:383-391`). Fixture f1: `uses → Child.__init__` at medium. A class with no `__init__` is audited as `class-without-explicit-init` (fixtures f1, f3).
- **Receiver shapes peeled** by `crate::receiver::peel`: `(f).m()`, `(await f).m()` and `Foo().m()` (a capitalised callee) mark the receiver as typed. Python is in `constructs_by_call` (`receiver.rs:46-51`). A PEP 8 private class `_Builder()` counts as a type (`receiver.rs:292-296`). Fixture f3: `Square().area()` binds high.
- **Chained call** `make().area()`: the receiver is the call text, which no type propagation uses (fixture f3, section 05).
- **Call operator** `obj()`: step 7 maps it to `type(obj).__call__` (`call_operator_names`, `cross_file.rs:394-397`). Fixture f1: `c()` → `Child.__call__` at medium.
- **Keyword arguments** are recorded in `RefRecord::kwargs` (`python.rs:829-839`) and narrow duck-typed fan-out through `signature_accepts` (`cross_file.rs:755`, used at `:6903`). Fixture f2: `o.kw(data=1)` binds only `K.kw` and not `L.kw(w, x)`.
- **`super().m()`** resolves through the base chain (section 06).
- **Value references** (callables passed as values):
  - In argument position of any call: `registrar::capture_value_refs` (`python.rs:316-322`). Tests `value_ref_in_ordinary_call_is_captured`, `value_ref_survives_an_unnameable_callee`.
  - On the right-hand side of an assignment (dicts, lists, tuples, sets, names, attributes, `a = b = f`, `x or f`, `a if c else b`): `capture_assignment_refs` (`python.rs:454-489`). Tests `value_refs_in_an_assigned_dict_are_captured`, `chained_assignment_names_its_value`, `value_ref_on_the_right_of_or_is_captured`.
  - In `return`/`yield`/bare decorator/default parameter/comprehension body/`is` comparison, only when `ctx.reference_sites` is set: `capture_value_positions` (`python.rs:349-442`). Test `value_positions_are_value_refs`.
  - Each kind is kept only when scoped: a name the file defines or imports, `self.x`/`cls.x`, or an attribute of an imported module (`retain_scoped_value_refs`, `python.rs:861-897`). Test `unscoped_value_positions_are_not_value_refs`.
  - Fixture f9 with `--reference-edges`: `reg(cb)`, `[cb]` and `sorted(key=cb)` each produce `top → cb` edges with via `reference`, at medium.
- **Registrar arguments** (route/handler in argument position): `registrar::capture`, gated on the per-language verb list (`registrar_verbs_for`, `rules.rs:11929`), with the call context `recv.name` (`python.rs:280-299`). Fixture f12: Django `path("u/", views.list_users)` is recognised.
- **Unnameable callee** (subscript callee `ConfigAttribute[x](…)` or a call on any other expression): no reference is recorded, but argument value refs are still captured (`python.rs:301-310`).

Not supported / partial:

- **Arity is not recorded**: Python is not in `OVERLOAD_ARITY_LANGS` (`type_hints.rs:1583`: java, scala, cpp only). Fixture f2: `z.f(1)` binds all three `Proc.f` overloads.
- **Subscript-dispatched calls** `handlers["a"]()` record no reference (fixture f2: no edge to `worker` from that call).
- **Operator dunders** `x + y` → `__add__`, `x[k]` → `__getitem__`, `len(x)` → `__len__`, and context managers (`with X() as y` → `__enter__`/`__exit__`) create no edges. Fixture f17: `ops` produces no edges to `X.__add__`/`__getitem__`/`__len__`. The dead-code dunder root rule covers liveness instead (section 12).
- **Paren-less calls**: Python has no paren-less call syntax, so `RefRecord::paren_less` does not apply.
- **Explicit generic calls**: not applicable (Python has no call-site type-argument syntax). `Foo[int]()` is a subscript callee and is not recorded.
- **Attribute value refs reached through a variable** (`handler.on_click` passed as a value) are not covered (CHANGELOG.md:752 "Not covered (WS6)").
- **Decorator applications** are recorded as calls (`@app.route("/x")` is a `call`), but at module or class level they have no enclosing callable.

## 04. Imports & module resolution

Supported:

- **Import forms parsed** (`parse_import`, `python.rs:899-935`):
  - `import a.b`, `import a.b as c`, `import a, b` become kind `import`. A comma list stays in `path`, and the alias is taken from the last item.
  - `from m import x`, `from m import x as y` and `from m import (a, b,)` (parentheses, multi-line, trailing comma) become kind `from-import`, with the item list in `alias`. Tests `imports_parsed`, `parenthesised_from_import_binds_every_name`.
- **Relative imports** `from . import x` and `from ..pkg import y` are made absolute by `ModuleIndex::py_abs` (`modules.rs:1075-1092`), used in `cross_file.rs:1626-1631`. Fixture f1: `from .. import core; core.helper(1)` and `from ..core import helper as h; h(2)` both bind `pkg.core.helper` at high.
- **Absolute dotted access** `import pkg.core; pkg.core.helper(3)` binds high (fixture f1).
- **Aliased import** `import json as j; j.dumps()` goes to the stdlib bucket (fixture f1).
- **Module index**: `modules.rs:190` (`indexed()` includes python), `:267-280` (python module/file and class index), `:377-397` (per-file `from` tables). `resolve_py` (`modules.rs:1901-1993`) handles:
  - Bare names from `from x import f`, through re-exports.
  - `mod.f()`, `pkg.mod.Cls().m()` and `pkg.Runner.m`.
  - `__init__.py` re-exports and `from .core import *` (a name only one star source provides; `py_lookup`, `modules.rs:1150-1214`).
  - Several same-named modules, narrowed to the caller's own directory (`py_split`, `modules.rs:1127-1146`).
  - Package attributes that shadow submodules (`py_package_binds`, `modules.rs:1097-1111`).
- **Step 0c** (written through an import of a workspace module) runs for Python (`cross_file.rs:5388`).
- **`not-in-imported-module`**: the refusal is recorded only when some workspace callable carries the name (`cross_file.rs:2810-2827`). Fixture f1: `core.missing()` with no `missing` anywhere goes to the per-file external bucket instead.
- **`FileFacts::packages`** holds the importable dotted module name (`python.rs:72-82`).
- **Workspace vocabulary** (`heads.rs`): Python names come from file path segments and the head of every callable's qualified name (`heads.rs:106, 189-193`). `py_workspace` treats relative modules as in-workspace and otherwise compares the first segment (`heads.rs:228-234`).
- **External-module refusal** (`heads.rs:414-440, 662`; covers `import` and `from-import` bindings):
  - Fixture f11: `requests.get("u")` with a workspace `get` present is recorded `external-module {module: requests}`.
  - Fixture f11: `from requests import get; get()` is likewise `external-module`.
  - `--include-external` mints `<external>::requests::get`.
- **Stdlib screen**: `os.getcwd()`, `print`, `len`, `super`, `getattr` and similar land in `stdlib_calls` (fixtures f1, f11). `--include-stdlib` mints `<stdlib>::os::getcwd` (fixture f11).
- **`if TYPE_CHECKING:` imports** are parsed like any import (fixture f2: no error). The walker records every `import_statement` wherever it sits (`python.rs:275-278`).

Not supported / partial:

- **Comma imports defeat framework detection**:
  - Fixture f12: `import argparse, signal` recognised neither `py-argparse` nor `py-signal`.
  - Split onto two lines, both were recognised.
  - The resolver's own import binding does split the comma list (`heads.rs:414-440`, `modules.rs:1512-1530`).
- **`__all__ += […]` and `__all__.extend(…)`** are deliberately not read (`python.rs:1202-1207`).
- **Packaging metadata** (`pyproject.toml`, `setup.cfg`, `setup.py` `entry_points`) is not read. The `py-setuptools-entrypoints` rule is a declared gap (`rules.rs:6900`).
- **Dynamic imports** `importlib.import_module("x")` and `__import__` create no edges. They are a dead-code `module-load` signal only (section 15).
- **Bare call to a name that is never imported**:
  - Fixture f11: `run()` in `main.py`, where only `lib.py`/`other.py` define `run`, bound `lib.run` at medium.
  - Python would raise `NameError`. Name-based fallback is still applied to bare calls.
- **Workspace module with a missing member** (`lib.nothere()`) was minted as `<external>::lib::nothere` under `--include-external` (fixture f11), although `lib` is a workspace module.
- **Stale audit record**: a from-import call that resolved cross-file (`hh()` → `lib.helper`, high) still leaves an intra-file `external_calls … no-candidate-in-file` record in the per-file audit (fixtures f1 `h`, f11 `hh`). Observed in the audit; whether the reconciliation should drop it is unverified.
- **`sys.path` manipulation, `.pth` files and namespace-package merging across directories** are not modelled. Only `__init__.py` chains and same-directory narrowing are (`modules.rs:1127-1146`).

## 05. Type inference & receiver typing

Supported:

- **Constructor assignment**: `x = Foo(…)` and `x = mod.Foo(…)` with a capitalised callee become `LocalType` (`infer_assignment_type`, `python.rs:552-595`). The `mod.Foo()` form is also pushed to `module_typed_locals`, which makes it a weak receiver type, never high (`python.rs:584-588`). Fixtures f3, f14: `y = base.Plain2(); y.inh3()` is medium.
- **Instance fields**: `self.x = Foo()` in any method types `self.x` in every method of the class (`emit_instance_field_types`, `python.rs:685-724`). A field assigned two different types is left untyped (`python.rs:614-636`). Tests `self_field_constructor_is_copied_onto_every_method`, `a_field_assigned_two_types_is_left_untyped_unless_annotated`. Fixture f4: `self.l.kw()` → `L.kw` high.
- **Class-level annotation** `x: Foo` and in-method `self.m: Foo = None` type the field, and win over assignments (`scan_class_annotations` `python.rs:1024-1079`, `record_annotated_self_field` `:643-680`). Tests `class_annotation_is_an_instance_field_type`, `nested_class_does_not_share_instance_field_types_with_same_named_outer`. Fixture f4: `self.k.kw()` and `self.m.kw()` → `K.kw` high.
- **Annotations through `declared_type`** (`annotation_type_stem`, `python.rs:1008-1019`; `cgg_core::declared_type`, `annotation.rs:35, 74`):
  - Stripped: `Optional[Foo]`, `Foo | None`, `"Foo"` / `'Foo'` (forward references), `Union[Foo, None]`, and the wrappers `ClassVar`, `Final`, `Annotated`, `InitVar`, `Required`, `NotRequired`, `ReadOnly` (`stdlib.rs:104-115`, `UNION_TYPE_CONSTRUCTORS` `:125`, `NULL_TYPE_MEMBERS` `:131-133`).
  - Left untyped: a union of real types and containers (`list[Foo]`).
  - Tests: `annotation.rs:178-259`, `annotation_stem_unwraps_optional_but_not_containers`, `container_annotation_does_not_type_the_field`.
- **Parameter annotations** `def f(p: Circle)` type `p` (`type_hints.rs:1421-1446`). Test `type_hints.rs optional_and_nullable_parameters_are_their_type`. Fixture f3: `t_param` → `Circle.area` high. Fixture f1: `svc: "Service"` (forward reference) types `self.svc`.
- **Return types** are parsed (`extract_return_type`, tests at `type_hints.rs:3482-3519`), but they are used only through the name heuristic below.
- **Name heuristics** (`type_hints.rs:823-867`), both marked weak:
  - Strategy 4: a receiver whose name is the lowercase or plural of a known return type, after a call to that function.
  - Strategy 2: a receiver whose name is the lowercase of any type name.
  - Fixture f6: `square = make(); square.area()` → `Square.area`.
- **Variable-name collisions with a class**: `own_file_class` (`cross_file.rs:6533-6542`) means a type the calling file declares binds only within that file's class and its bases.
- **Kwargs-based narrowing**: see section 03.

Not supported / partial (fixtures f3, f4 unless noted):

- **Return type through a call or a non-matching local**:
  - `make().area()` and `y = make(); y.area()` with `def make() -> Square` fan out to `Square.area` and `Circle.area` at medium.
  - The return type is never applied to a chained call or to a local whose name differs.
- **Local variable annotation** `sq: Square = get(); sq.area()` is not used: it fans out to both. The plugin reads only `self.x: T` and class-level `x: T`.
- **Not typed at all**: `with Circle() as cm`, `for x in xs: list[Circle]`, `typing.cast(Circle, o)`, `isinstance(o, Circle)` narrowing, walrus `(w := L())`, tuple unpacking `a, b = L(), K()`, and module globals `G = L()` used in a function. Each fans out or is unresolved. Python has no entry in `CAST_INTRINSICS` (`stdlib.rs:35-46`).
- **False high from the variable-name heuristic** (a wrong edge):
  - Fixture f5: `def t(k): k.kw()` binds `K.kw` at **high**, and `def u(l): l.kw()` binds `L.kw` at high, from the parameter's name alone.
  - Fixture f4: `match o: case L() as k: k.kw()` binds `K.kw` at **high**, which is wrong.
  - Fixture f6: `circle = obtain(); circle.area()` binds `Circle.area` high, although `obtain` returns nothing typed.
  - Strategy 2 (`type_hints.rs:863-867`) marks these weak. The intra-file linker still emitted `high`.
- **Closure parameter typing** (`scoped_types`, `settle_closure_args`) is Dart/C#/Scala only. Not applicable: Python lambdas take no annotations.
- **Implicit receivers** (`settle_implicit_receivers`) are Kotlin only. Not applicable.
- **Generics and `TypeVar`**: `Generic[T]` is excluded as a base (`dispatch.rs:443-460`). A `T`-typed parameter gives no owner.

## 06. Inheritance, dispatch & overloading

Supported:

- **Class records**: every class pushes a `ClassDecl` with its bases (`attrs::base_types`), even without methods (`python.rs:211-223`). Test `every_class_records_its_bases_even_without_methods`. Each method's `DefRecord::base_types` carries its class's bases (`python.rs:547`). Test `underscore_prefixed_subclass_records_base_types`.
- **`super().m()`** resolves through the base chain, cross-file, at high when anchored. Fixture f14: `super().inh3()` → `pkg.base.Plain2.inh3` high. If the chain leaves the tree, it is refused `super-base-out-of-graph` (fixture f2: `super().__init__()` with base `ABC`). The CLAUDE.md list of every-class-recorded languages includes Python.
- **Same-file inherited `self.m()`** binds high (fixture f14 `same.S0.go → same.Plain.inh`).
- **Typed receiver reaching an inherited method cross-file**: `def q(x: S6): x.inh3()` → `Plain2.inh3`. Medium when the subclass is in another file (fixture f14 sub4). High when the receiver type is imported and declares the member (fixture f14 sub5).
- **`cls.m()`** resolves to the class's own member (fixture f14: `S5.c → S5.inh3` high).
- **Inheritance fan-out** (`dispatch::inheritance_fanout`): Python is in `INHERITANCE_LANGS` (`dispatch.rs:110-123`). The base→override edge is opt-in with `--dynamic-dispatch` and always computed under `--dead-code`.
  - Fixture f9: `Base.run → A.run` and `Base.run → B.run`, `low`, via `dynamic`. Async overrides are included (fixture f19).
  - Unlike other languages, a `@staticmethod` still overrides (`dispatch.rs:388-392`).
  - Tests: `resolve.rs python_inheritance_fanout_is_opt_in`, `python_override_walks_past_a_base_that_does_not_define_the_method`, `python_subclass_override_is_live_when_base_method_is`, `python_override_through_a_bodiless_intermediate_class_is_live`.
- **Bases ignored for inheritance**: `object`, `type`, `Enum`/`IntEnum`/`StrEnum`, `ABC`/`ABCMeta`, `Generic`, `Protocol`, `TypedDict`, `NamedTuple`, `Exception`/`BaseException` (`dispatch.rs:443-460`).
- **Protocol, ABC and `@abstractmethod` members** are dropped from duck fan-out when a concrete candidate survives (`cross_file.rs:1153-1190, 6880`). Fixture f2: `g.greet()` binds `En.greet` and `Fr.greet`, not `Greeter.greet`. `s.area()` drops the abstract `Shape.area`.
- **Bare calls never reach a method** (`bare_call_is_never_a_method`, `intra_file.rs:49-53`; cross-file `cross_file.rs:4137-4139`). Fixture f18: `am()` → `not-in-scope-for-bare-call`.

Not supported / partial:

- **Inherited `self.m()` across files does not resolve** (major gap):
  - Fixture f13: `class S1(Plain)` with `from base import Plain`; `self.inh()` → unresolved `candidates-in-other-files {candidates: 1}`.
  - Fixture f14: the same with `from pkg.base import Plain2` and `from .base import Plain2` (sub, sub2).
  - Fixture f2: `self.helper_inherited()`.
  - Root cause unverified. `S1().inh()` from the same file resolves, at medium.
- **Overload arity narrowing**: Python is not in `OVERLOAD_ARITY_LANGS` (`type_hints.rs:1583`). `@overload` stubs are not dropped as declarations (only `abstractmethod`/`abstract`/Protocol are). Fixture f2: `z.f(1)` → three `Proc.f` edges.
- **`member_of_calling_class` / `overrides_below`** for bare calls are the Java/C#/Ruby/Scala/Dart paths. Python bare calls cannot reach methods by design.
- **Mixins and multiple inheritance**: bases are recorded in order, but the C3 MRO is not computed. `one_declaring_ancestor` refuses `high` when several ancestors declare the member (`cross_file.rs:4654`).
- **Extension methods and traits**: not applicable.
- **Metaclass keyword** `class X(Base, metaclass=M)`: presumably dropped from bases by `is_type_name` (`attrs.rs:199-240`). Unverified.
- **Dynamically injected methods** (`setattr(cls, …)`, class decorators adding members, `__getattr__` forwarding) are not modelled.

## 07. Scoping & shadowing

Supported:

- **`FileFacts::locals`** comes from `PY_SPEC` (`locals.rs:183-204`, selected at `:383`).
  - Binders: `parameters`, `lambda_parameters`, `default_parameter`, `typed_parameter`, `typed_default_parameter`, `assignment.left` (scope `Decl`), `for_statement.left` (scope `Own`) and `as_pattern_target`.
  - Function scopes: `function_definition`, `lambda`. Tuple, list and splat patterns are destructured.
- **Shadowed by local**: a bare call through a parameter or local is never another file's function. Fixture f11: `def f(helper): helper()` → `shadowed-by-local`. Fixture f15: `except … as helper` and `with open() as run` also shadow.
- **Lexical narrowing of nested definitions**: Python is in `lexically_scoped` (`intra_file.rs:119-135`). For Python the rule also applies to `dot_call` refs (`intra_file.rs:350-370`). Fixture f1: `outer → outer.inner`. Fixture f11: two same-named `inner` functions each bind their own. Test `resolve.rs python_bare_call_binds_by_lexical_scope`.
- **Receiver headed by a local** is not treated as an import (`heads.rs:681-683`, `shadows_receiver_head`).

Not supported / partial (fixture f15):

- **Comprehension variables** are not binders. `[helper() for helper in xs]` binds the module `helper` at high (wrong).
- **`for` loop variables after the loop**: `Own` scope ends with the loop, but Python leaks the variable to function scope. `for helper in []: …` then `helper()` binds the module `helper` high.
- **`global` / `nonlocal`** are not modelled. They are harmless here (`global run; run()` binds the module `run`).
- **Positional shadowing**: the `Decl` scope starts at the assignment, so `helper = helper()` still binds the module function (right in practice). A call before a later local assignment in the same function (`run(); run = 3`) also binds, although Python would raise `UnboundLocalError`. Positional shadowing as in OCaml/F#/Lua is not applied.
- **Conditional definitions**: no `conditional` flag. `DefRecord::conditional` is PHP only (CLAUDE.md). Both `if`/`else` definitions of `cond` bind, producing duplicate medium edges (fixture f2).
- **Class-body scope**: class attributes are not locals. Methods cannot see class-body names in Python, which cgg mirrors only incidentally.

## 08. Macros, preprocessor & codegen

- Not applicable: Python has no macros, preprocessor or templates. `macro:<name>` tagging is Rust/C/C++ only.
- `FileFacts::name_mentions` is not populated by the Python plugin (grep: no `name_mentions` in `python.rs`).
- **Decorators as codegen**: decorators are recorded as attributes and may root a callable (sections 10, 12). The functions they generate (`functools.wraps` wrappers, `dataclass` `__init__`, `property` accessors) are not synthesised.
- **Dataclass and attrs generated `__init__`/`__eq__`/`__repr__`** are not synthesised. `Foo(…)` on a `@dataclass` with no `__init__` is audited `class-without-explicit-init` (same mechanism as fixture f3).
- **`typing.TYPE_CHECKING` blocks** are parsed like ordinary code. No `#if`-style evaluation.

## 09. FFI & cross-language links

Supported:

- **PyO3** (Rust `#[pyfunction]`/`#[pymethods]`/`#[pyclass]` → Python): the family `pyo3` is classified at `ffi.rs:657`, and its allowed source language is `["python"]` (`ffi.rs:690-699`).
  - Pass B links a Python callable with the same simple name (and matching owner) to the Rust export at medium (`ffi.rs:188-260`).
  - Fixture f9 with a `mymod.pyi` stub declaring `rust_add`: `caller.py_calls → mymod.rust_add` (high) and `mymod.rust_add → crate::rust_add` (medium, via `ffi pyo3`).
  - Tests `ffi.rs pyo3_cross_language_edge`, `pyo3_export_links_a_python_caller_of_the_same_name`.
- **Kivy KV → Python**: `.kv` `on_release: root.do_it()` → `server.Root.do_it`, high, via `ffi` (`link_kivy_python`, `ffi.rs:270-560`). It uses Python `class_fields` and `App.build` return constructors (`kivy_app_context`, `ffi.rs:485-540`). Fixture f10. Tests `crates/cgg/tests/kivy.rs kv_on_release_resolves_to_python_method` and siblings.
- **Protobuf rpc → Python servicer**: `Greeter.SayHello → server.GreeterServicer.SayHello`, low, via `descriptor grpc`. The match is by method name plus an owner containing the service name, language-agnostic (`descriptor.rs:95-145`). Fixture f10.
- **Rust `#[pymethods]` container**: `container_attributes_for("ffi-export","rust")` (`rules.rs:11770-11777`) roots the Rust side. The Python side gets nothing extra.

Not supported / partial:

- **PyO3 without a Python definition**: fixture f9 without the stub has `rust_add(1)` / `mymod.rust_add(2)` and no FFI edge. Pass B links callables, not call sites (`ffi.rs:197-210`).
- **`ctypes` / `cffi` → C** (`CDLL(...).func()`): the `c-abi` family accepts only C-family callers (`ffi.rs:698`). No Python→C edge. Unverified by fixture.
- **Cython (`.pyx`/`.pxd`)**: not detected (not in the extension list).
- **wasm-bindgen, napi, JNI, P/Invoke, asm**: not applicable to Python callers.
- **Markup code-behind** (`markup.rs`: xaml/razor/tscn): C# only. The Kivy link above is Python's markup equivalent.

## 10. Frameworks & entry points

Every rule in `SPECS` with `language: "python"`, extracted from `crates/cgg-core/src/frameworks/rules.rs` by `rules.py` in the scratch directory. The 49 rules are listed in file order. Format: id, kind (TrustKind), and trigger (`detect` import prefixes; then attrs = decorator keys, regs = registrar verbs, bases = base classes, methods = method names). "node:false" means root-mark only. "gap" means the rule only reports coverage.

| line | id | kind | trigger |
| --- | --- | --- | --- |
| 94 | fastapi | network | detect `fastapi`. attrs get/post/put/delete/patch/head/options/route/api_route/websocket/middleware/exception_handler. regs add_api_route/add_route/add_websocket_route |
| 124 | flask | network | detect `flask`. attrs route/get/post/put/delete/patch/errorhandler/before_request/after_request/teardown_request/websocket. regs add_url_rule. bases MethodView/Resource with methods HTTP_VERBS |
| 153 | django | network | detect `django`, paths `urls.py`. attrs require_http_methods/api_view/login_required/csrf_exempt. regs path/re_path/url/register. bases View…ReadOnlyModelViewSet (12) with methods get/post/…/destroy |
| 205 | django-admin | cli | detect `django.core.management`. bases BaseCommand/AppCommand/LabelCommand. methods handle/add_arguments |
| 222 | celery | queue | detect `celery`. attrs task/shared_task/periodic_task/on_after_configure. bases Task. methods run |
| 239 | click | cli | detect `click`, `typer`. attrs command/group/callback |
| 256 | torch | lifecycle (node:false) | detect torch/pytorch_lightning/lightning. bases nn.Module/Module/LightningModule/Dataset/IterableDataset. methods forward/training_step/…/`__getitem__`/`__len__` |
| 290 | kivy | lifecycle (node:false) | detect `kivy`. regs bind/fbind/schedule_once/schedule_interval. bases App/Widget/… (34). methods build/on_*(45). observer_types*Property (14). handler_on_enclosing_owner = true |
| 1484 | tornado | network | gap (no matchers) |
| 1501 | aiohttp | network | gap |
| 1518 | sanic | network | gap |
| 1535 | starlette | network | gap (detect starlette.applications/endpoints) |
| 1552 | bottle | network | gap |
| 1569 | pyramid | network | gap |
| 1586 | airflow | schedule | gap |
| 1603 | prefect | schedule | gap |
| 1620 | dagster | schedule | gap |
| 1637 | streamlit | network | gap (no matchers) |
| 1654 | gradio | network | gap |
| 1671 | aws-lambda-powertools | network | attrs get/…/route/not_found/exception_handler/inject_lambda_context/capture_lambda_handler/log_metrics/batch_processor/async_batch_processor/event_source/event_parser/validator. regs process_partial_response(_async) |
| 4043 | pydantic | lifecycle (node:false) | attrs model_validator/field_validator/validator/root_validator/field_serializer/model_serializer/computed_field/model_post_init. methods model_post_init |
| 4078 | aws-lambda | network | detect aws_lambda_powertools/aws_lambda_typing/awslambdaric. methods lambda_handler |
| 4260 | aws-cdk | network | detect aws_cdk. regs Function/DockerImageFunction/SingletonFunction/NodejsFunction/PythonFunction/GoFunction. string_targets |
| 4378 | azure-functions | network | detect azure.functions. attrs route/function_name/timer_trigger/blob_trigger/queue_trigger/service_bus_*_trigger/event_hub_message_trigger/event_grid_trigger/cosmos_db_trigger/schedule |
| 4549 | firebase-functions | network | detect firebase_functions. attrs on_request/on_call/on_document_*/on_value_*/on_message_published/on_schedule/on_object_finalized |
| 4774 | gcp-functions | network | detect functions_framework. attrs http/cloud_event/typed/errorhandler |
| 6784 | py-atexit | lifecycle | detect atexit. regs atexit.register |
| 6801 | py-signal | lifecycle | detect signal. regs signal/add_signal_handler |
| 6818 | py-threading | lifecycle (node:false) | detect threading/multiprocessing/concurrent.futures. regs Thread/Process/Timer |
| 6835 | py-unittest | test | detect unittest. bases TestCase/unittest.TestCase/IsolatedAsyncioTestCase/AsyncTestCase/FunctionTestCase. methods test*/setUp/tearDown/setUpClass/tearDownClass/asyncSetUp/asyncTearDown |
| 6866 | py-argparse | cli | detect argparse. regs set_defaults |
| 6883 | py-context-manager | lifecycle (node:false) | detect contextlib. methods `__enter__`/`__exit__`/`__aenter__`/`__aexit__` |
| 6900 | py-setuptools-entrypoints | cli | detect setuptools/pkg_resources. gap (packaging metadata not read) |
| 8052 | litestar | network | attrs get/post/put/patch/delete/head/options/route/asgi/websocket/websocket_listener/websocket_stream |
| 8082 | falcon | network | regs add_route/add_static_route/add_sink/add_error_handler. methods on_get/…/on_websocket/process_request/process_resource/process_response |
| 8116 | quart | network | detect quart/quart_cors/quart_schema. attrs route/HTTP verbs/websocket/errorhandler/before_*/after_*. regs add_url_rule/add_websocket |
| 8150 | connexion | network | detect connexion. No matchers. Gap text is a non-literal (unverified) |
| 8167 | robyn | network | attrs HTTP verbs/websocket.on/before_request/after_request/startup_handler/shutdown_handler/exception. regs add_route/add_web_socket |
| 8200 | nicegui | network | gap |
| 8217 | panel | network | gap |
| 8234 | dash | network | detect dash + dash_* components. attrs callback/clientside_callback/long_callback/celery_callback |
| 8261 | scrapy | lifecycle (node:false) | bases Spider/CrawlSpider/XMLFeedSpider/CSVFeedSpider/SitemapSpider/scrapy.Spider. methods parse/start/start_requests/parse_start_url/closed/spider_opened/spider_closed/from_crawler |
| 8294 | huey | queue | attrs task/periodic_task/db_task/db_periodic_task/lock_task/on_startup/on_shutdown/signal/pre_execute/post_execute |
| 8322 | rq | queue | detect rq/django_rq. attrs job. regs enqueue/enqueue_call/enqueue_at/enqueue_in/enqueue_many/schedule. string_targets |
| 8346 | dramatiq | queue | detect dramatiq/django_dramatiq. attrs actor/dramatiq.actor |
| 8363 | apscheduler | schedule | detect apscheduler/django_apscheduler/flask_apscheduler. attrs scheduled_job/task. regs add_job. string_targets |
| 8380 | pytest | test | detect pytest/_pytest/pytest_asyncio. attrs fixture/pytest.fixture. methods test*/setup_module/…/teardown_class. (Its `gap` text still says "cgg has no entry rules for pytest", although matchers exist) |
| 9288 | marimo | lifecycle | detect marimo. attrs cell/function/class_definition |
| 9307 | sqlalchemy-events | lifecycle | detect sqlalchemy. attrs listens_for |

Helper tables (`rules.rs` after line 11627):

- `member_sources_for`: no Python entries (Java/C# only; `rules.rs:11719-11760`).
- `signature_methods_for`: no Python entries (Java only; `rules.rs:11835-11860`).
- `container_attributes_for`: no Python entries (Rust only; `rules.rs:11770-11797`).
- `markup_kinds_for`: no Python entries (C# only; `rules.rs:11812-11820`). Kivy KV is handled in `ffi.rs` instead.
- `visibility_entries_for`: Solidity only (`rules.rs:11686-11691`).
- `self_module_markers_for`: JS/TS only.
- `TEST_CLASS_MEMBERS_ARE_ROOTS` includes `python` (`rules.rs:11806`). See section 11.
- `registrar_verbs_for("python")` is the union of the `regs` columns above (`rules.rs:11929-11955`).
- `observer_types` and `handler_on_enclosing_owner` are used only by the `kivy` rule (`rules.rs:290`). Tests `frameworks/mod.rs` at `:2791` and `:3122` (`on_text` observer).
- Dead-code built-in attribute roots that apply to Python decorators (`FRAMEWORK_ATTRS`, `roots.rs` after `:172`): `app.route`, `route`, `get`, `post`, `put`, `delete`, `patch`, `pytest.fixture`, `fixture`, `click.command`, `click.group`, `celery.task`, `task`, `command`, `event_handler`. These root by attribute text even without the framework import.

Fixtures and tests:

- Fixture f7:
  - flask `@app.route` → `<framework-entry>::network::flask::route("/x")`
  - click → `<framework-entry>::cli::click::app.cli`
  - celery `@shared_task` → queue entry
  - `atexit.register(on_exit)` → lifecycle entry
  - unittest/pytest harness nodes
- Fixture f12: django (urls.py `path()` + `View.get`), fastapi, pydantic, rq (string target `"api.bg"`), torch, py-argparse and py-signal (once imports were split) recognised.
- Fixture f2: py-threading root-marked (`Thread(target=worker)`).
- Tests (`crates/cgg/tests/frameworks.rs`): `shape_a_python_flask_route_is_a_network_entry`, `django_as_view_binds_to_the_classes_http_methods`, `aws_lambda_python_handler_convention_and_powertools_routes`, `firebase_python_triggers_are_entries`. Also `frameworks/mod.rs` tests at `:2475-2570` (flask gating on import, torch `forward`).

Not supported / partial:

- **Gap-only rules** enumerate nothing: tornado, aiohttp, sanic, starlette, bottle, pyramid, airflow, prefect, dagster, streamlit, gradio, nicegui, panel, connexion, py-setuptools-entrypoints.
- **Comma-import detection**: `import a, b` defeats rule detection (fixture f12, section 04).
- **Django class-based views reached via `as_view()`** are covered by the test above. Other URL-confs (`include()` strings) are unverified.

## 11. Test harness entries

Supported:

- **`py-unittest`** (kind test): one node `<framework-entry>::test::py-unittest` with entry edges to each `test*`/`setUp`/`tearDown`/… method of a `TestCase` subclass (`rules.rs:6835`). Fixture f7: `TestThing.setUp` and `TestThing.test_a` are rooted via `framework:test-harness`.
- **`pytest`** (kind test): node `<framework-entry>::test::pytest`. It covers `test*` functions and methods, the xunit hooks and `@pytest.fixture`/`@fixture` (`rules.rs:8380`). Fixture f7: `test_app.fx` and `test_app.test_b` are rooted.
- **Pattern semantics**: `test*` is a case-sensitive prefix pattern. A name-only test rule applies only in a test file or a file importing the harness (CLAUDE.md, phase 6).
- **`TEST_CLASS_MEMBERS_ARE_ROOTS`** includes Python (`rules.rs:11806`; applied at `roots.rs:506-535`, `frameworks/mod.rs:898`). Every other member of a test class becomes a test root mark. Fixture f7: `TestThing.helper` is rooted via `framework:rule test:py-unittest:base-type`. Test `frameworks/mod.rs:2435`.
- **Plugin `test_role`** (`py_test_role`, `python.rs:1182-1200`):
  - `@pytest.fixture`/`@fixture` → Fixture.
  - `@pytest.mark.*` → Case.
  - The `PY_FIXTURES` names (`setUp`, `tearDown`, `setUpClass`, `tearDownClass`, `setup_/teardown_` + module/function/class/method; `python.rs:1162-1175`) → Fixture.
  - `test_*` → Case.
- **Test-file classification** (`testfile.rs:79-83`): `test_*.py`, `*_test.py` and `conftest.py`. Test `testfile.rs:154-166`. Test files are withheld from dead-code findings unless `--include-tests` is passed (fixture f7).
- **Dead-code test-name fallback**: `is_test_name("python")` is `test_*`, `setUp` or `tearDown` (`roots.rs:984`).
- **`--no-test-entries`** turns the harness nodes into root marks. Fixture f7: 5 framework-entry lines instead of 6 with the flag.

Not supported / partial:

- **`@pytest.mark.parametrize` on a non-`test_` function** gets `test_role = Case` but no root. Fixture f7 with `--include-tests`: `check_param` is a medium finding (consistent with pytest's own collection).
- **Not rooted as hooks**: conftest hooks (`pytest_addoption`, `pytest_collection_modifyitems`, any `pytest_*`), fixtures matched by parameter name, `pytest11` plugins and `indirect=True` parametrisation. The `pytest` rule's gap text says so (`rules.rs:8380-8405`). The `fixture` attribute does root the fixture definition itself.
- **Not covered**: doctests, `nose`/`nose2` generators, `hypothesis` `@given` without a `test_` name, and `asyncio`-marked tests outside `pytest_asyncio`. No rule; unverified by fixture.
- **Benchmark and fuzz harnesses** (pytest-benchmark, atheris): no rule.

## 12. Dead-code analysis

Supported:

- **Signals**, all `full` in the capability table (fixture f7: `capabilities` row for python): visibility, attributes, value_references, dispatch, exports, test_tagging. Python's `max_confidence` is `high`.
- **Visibility** (`py_vis`, `python.rs:1151-1159`): dunder names are Public, a leading `_` is Private, anything else is Public. Private unreferenced definitions report `high` (fixture f7: `_private_never`, `Plugin.__mangled`). Public ones report `medium` with `public-visibility` evidence.
- **Exports**: `__all__` literal list/tuple (`py_dunder_all`, `python.rs:1208-1247`) gives root kind `ExportedApi`. Fixture f1: `--why-live exported_only` reports `[ExportedApi]`.
- **Roots**:
  - Program entry `main` (`roots.rs:20` `UNIVERSAL_ENTRY`; fixture f12 `api.main` `[ProgramEntry]`).
  - Top-level invocation (`toplevel:invocation`; fixture f7 `on_exit`, `holder`).
  - Dunder methods as runtime lifecycle roots (`DUNDER_LANGS` includes python, `roots.rs:160-161`; fixture f7 `Model.__str__`/`__repr__`).
  - Framework attributes, framework rules and test harness (section 10/11).
  - Inheritance fan-out under `--dead-code` (`config.dynamic_dispatch: true` in fixture f7 JSON).
- **`implements-external-contract`** caps a finding at medium. Python marks nothing, so any non-function member of a class with an outside base qualifies, except name-mangled `__m` (`deadcode/mod.rs:500-530`). Fixture f7: `Plugin.hook` medium with `implements-external-contract:ExtBase`. Test `deadcode/mod.rs:2299-2318`.
- **Value references**: a function returned or passed as a value is live (fixture f7: `passed_value` has no finding).
- **Reflection cap**: a `getattr`/`setattr`/`hasattr`/`delattr` literal caps a same-named finding at medium (`name-collides-with-screened-site:reflection`). A module-load literal (`import_module`) does not cap (`:module-load`). Fixture f8: `_by_getattr` medium, `_by_import` high.
- **Blind spot printed**: "getattr/setattr and other string-dispatched calls" (`deadcode/caps.rs:53-56`).

Not supported / partial:

- **Unreachable-code signal is extracted but not consumed**:
  - The plugin computes `FileFacts::unreachable` with the `cfg::PYTHON` terminators: return, raise, break, continue (`cfg.rs:79-90`; `python.rs:118-121`).
  - Grep finds no reader of `FileFacts::unreachable` in `cgg-resolve` or `cgg/src`; only the `signals().unreachable` flag is copied (`lib.rs:1935`).
  - Fixture f8: `_cb()` after `return 1` still makes `_cb` live (`--why-live` shows a High hop).
- **Async methods miss the external-contract cap**: they are recorded with kind `function`, and the cap requires `nd.kind != CallableKind::Function` (`deadcode/mod.rs:505`). Fixture f20: `async def _ahook` in a class with outside base `ExtBase` is reported **high** "unreferenced function". The sync sibling `_shook` is medium with `implements-external-contract:ExtBase`.
- **Arbitrary string literals** naming a function (`x = "_by_str"`) give no doubt. Fixture f8: `_by_str` high. This is intended: only the reflective positions in `dynuse.rs` count.
- **`.pyi` stub shadows its implementation**: fixture f16, `mod.f` calls bind to `mod.pyi`'s `f` only. The implementation in `mod.py` gets no edge, though it was not reported dead in that fixture.
- **`__all__ += …` / `.extend`** are not read (section 04).
- **Conventional root names**: `CONVENTIONAL_METHODS` (`initialize`, `to_s`, `toString`, `equals`, `hashCode`, `Dispose`, `describe`) are cross-language and not Python-specific (`roots.rs:162-170`).

## 13. Builtins & stdlib tables

- **`crates/cgg-core/src/stdlib/python.txt`**: 1,180 names (`grep -c .`), included at `stdlib.rs` (`static PYTHON`). Builtins (`len`, `print`, `super`, `getattr`) and module members (`join`, `dumps`, `getcwd`) are screened to the `stdlib` bucket. Fixtures f1, f11.
- **`MAYBE_TYPE_WRAPPERS["python"]`**: Optional, ClassVar, Final, Annotated, InitVar, Required, NotRequired, ReadOnly (`stdlib.rs:104-115`).
- **`UNION_TYPE_CONSTRUCTORS`**: `Union` (`stdlib.rs:125`).
- **`NULL_TYPE_MEMBERS`**: `None`, `NoneType` (`stdlib.rs:131-133`).
- **Constructor and call-operator names**: `constructor_names("python") = ["__init__"]`, `call_operator_names = ["__call__"]` (`cross_file.rs:383-397`).
- **`DYN_MODULE_LOADERS`** includes `import_module` (`facts.rs:165`).
- **Inheritance-ignored bases**: see section 06 (`dispatch.rs:443-460`).
- **Not populated for Python**: `CAST_INTRINSICS`, `CONSTRUCTING_METHODS` (Ruby `new` only), `CALLBACK_TYPE_CONSTRUCTORS` (Python `Callable[[A], R]` is not used to type closures), `TOP_TYPES` (no `object`/`Any` entry), implicit-package tables (Scala only), and `rust_types`-style wrapper tables.
- **`--include-stdlib` / `--include-external`** mint `<stdlib>::os::getcwd` / `<external>::requests::get` leaf nodes (fixture f11). With `--include-external`, the leaf for `o.m()` on a parameter receiver was minted as `<external>::o::m`, although `o` is a local, not a module (fixture f11).

## 14. Confidence & certainty

`high` is reachable for Python through:

- **Intra-file unique binding**: fixtures f1, f3 (`t_param`, `t_ctor`, `t_self_chain`), f4 (self-field types).
- **Import-anchored cross-file binding**: from-import, relative import and dotted module path through `ModuleIndex::resolve_py` (`Scoped::Bound { certain: true }`, `modules.rs:1914-1959`). Fixture f1: all `pkg.sub.mod.caller` edges are high.
- **Typed receivers whose type is anchored** (`CertainCtx::anchored`, Python arms at `cross_file.rs:4609-4620`):
  - A dotted type `pd.Series` must match the owner path.
  - Otherwise the type must be declared in the calling file, or imported from the candidate's own package.
  - Base anchoring for Python requires the base to be declared in the caller's file (`base_anchored`, `cross_file.rs:4525-4540`). Inherited and super bindings also need `heads.own_classes` and `bases_anchored` (`cross_file.rs:6514-6526`).
  - Test `resolve.rs python_typed_receivers_are_high_only_when_the_type_is_anchored`, `python_super_through_a_dependency_base_is_not_high`. Fixture f14: `q(x: Plain2)` high, `super().inh3()` high.

Forced below high:

- **`weak_receiver_type`**: `x = mod.Cls()` via `module_typed_locals` gives medium (fixture f14 sub5 `y.inh3()`).
- **By-name duck fan-out**: always medium (fixtures f2, f3).
- **Constructor step 6 and call-operator step 7**: medium (fixture f1 `Child.__init__`, `Child.__call__`).
- **Inheritance fan-out** edges are `low` (fixture f9). Descriptor links are `low` (fixture f10). PyO3 Pass B edges are `medium` (fixture f9).
- **Minified callers** stay medium (`cross_file.rs:4545-4550`).
- **Mermaid marking**: an arrow with no high site gets a leading `?` label. Fixture f1: `N16 -->|?| N4` (`uses → Child.__init__`).

Defect:

- **False high from the name heuristic**: the variable-name type heuristic (`type_hints.rs:863-867`, Strategy 2, and Strategy 4 at `:823-861`) marks the site weak, but the intra-file link still emits `high`. Fixtures f4, f5, f6: `k.kw()` → `K.kw` high from a param named `k`, `case L() as k` → `K.kw` high, `circle = obtain()` → `Circle.area` high. Section 05 has the full list.

## 15. Dynamic use & reflection

- **`dynuse::REFLECTIVE["python"]`** covers `getattr`, `setattr`, `hasattr`, `delattr` and `import_module` (`dynuse.rs:26-29`). An identifier-shaped literal in argument position is recorded as a `DynUse` when `ctx.deadcode_signals` is set (`python.rs:122-124`). Tests at `dynuse.rs:160-178, 248-250`.
- **Effect**:
  - Suppression only, never an edge (`facts.rs` `DynUse` docs, `:150-170`).
  - Reflection caps same-named findings at medium. `import_module` is a module load and does not cap (`DYN_MODULE_LOADERS`, `facts.rs:165`).
  - Fixture f8: `_by_getattr` medium (`reflection`), `_by_import` high (`module-load` risk only).
- **Caveat**: per CHANGELOG.md:752-755, a reflective lookup caps every same-named callable of the language, not only those of the receiver's type.

Not supported:

- **`__import__`, `eval`, `exec`, `compile`, `operator.methodcaller`, `operator.attrgetter`**: not in the table (`dynuse.rs:26-29`).
- **Non-literal reflection**: `getattr(o, name)` with a variable or an f-string records nothing (`is_identifier_shaped` needs a literal, `dynuse.rs:47-57`).
- **`__getattr__` / `__getattribute__` forwarding, `setattr(cls, …)` method injection, `globals()[name]()`, `vars()`**: no handling.
- **`importlib.import_module("x").f()`**: no edge, a dead-code signal only.

## 16. Known gaps & limitations

Confirmed by fixture in this pass:

1. **Inherited `self.m()` across files never resolves** and is audited `candidates-in-other-files` (fixtures f2, f13, f14). It works same-file, and for typed receivers and `super()`.
2. **False `high` edges from the variable-name type heuristic** (fixtures f4, f5, f6): `def t(k): k.kw()` → `K.kw` high; `case L() as k: k.kw()` → `K.kw` high (wrong class); `circle = obtain()` → `Circle.area` high.
3. **Typing gaps**: return types are not applied to chained calls or differently named locals; local variable annotations (`x: Foo = …`) are ignored; `with … as`, `for` over `list[Foo]`, `typing.cast`, `isinstance` narrowing, walrus, tuple unpacking and module globals are not typed (fixtures f3, f4).
4. **Unreachable-code regions are extracted but never consumed**. A call after `return` keeps its target live (fixture f8; grep shows no reader).
5. **Async methods have graph kind `function`**, so they miss the Python `implements-external-contract` cap and report high (fixture f20).
6. **A `.pyi` stub next to its `.py` captures every edge**. The implementation gets none (fixture f16).
7. **`import a, b` comma imports defeat framework detection** for both modules (fixture f12).
8. **Scoping**: comprehension variables do not shadow, and a `for` variable used after its loop does not shadow (fixture f15).
9. **Conditional `if`/`else` definitions** both bind, producing duplicate medium edges (fixture f2). There is no `conditional` flag (PHP only).
10. **No arity narrowing for `@overload`**: all stubs and the implementation bind (fixture f2).
11. **Callables not extracted**: class-body method aliases (`run = _run`), `@x.setter` (classified as a plain method with the getter's name) and anonymous lambdas (fixture f2).
12. **No edges for operator dunders, subscripts, `len()` or context managers** (fixture f17).
13. **PyO3 links Rust exports only to same-named Python callables (stubs), never to call sites** (fixture f9).
14. **Bare calls to never-imported names fall back to name matching** at medium (fixture f11 `run()` → `lib.run`).
15. **`--include-external` mints `<external>::` leaves** for local-headed receivers (`<external>::o::m`) and for missing members of workspace modules (`<external>::lib::nothere`) (fixture f11).
16. **Audit quirk**: a from-import call that resolved cross-file keeps a stale intra-file `external_calls` record (fixtures f1, f11).

From code comments, CHANGELOG and README:

- `__all__ += […]` / `.extend()` deliberately not read (`python.rs:1202-1207`).
- Python attribute value refs reached through a variable (`handler.on_click`) are not covered (CHANGELOG.md:752).
- The module docstring is stale: it says "Module name … derived from the file stem for Task 4 … stack-graphs". Since then `module_name` walks `__init__.py` (`python.rs:17-19` vs `:139-159`), and stack-graphs is a no-op (CLAUDE.md).
- The `pytest` rule's `gap` text claims "cgg has no entry rules for pytest" although the rule has attrs and methods (`rules.rs:8380-8405`).
- Gap-only framework rules (section 10): tornado, aiohttp, sanic, starlette, bottle, pyramid, airflow, prefect, dagster, streamlit, gradio, nicegui, panel, connexion, py-setuptools-entrypoints.
- README "Limitations": type inference is partial and "does not handle generics or fully dynamic typing".
- Ignored tests: `grep '#\[ignore\]'` across `crates/` returned nothing.
- **Unverified**: `ctypes`/`cffi` linkage (no fixture run); metaclass keyword filtering in bases; `%%bash` cell bodies being parsed as Python.

## Summary scorecard

| section | support | note |
| --- | --- | --- |
| 01 Detection & parsing | full | .py/.pyi/.ipynb + shebangs. Notebook magic stripping is line-prefix only |
| 02 Callables extracted | partial | functions/methods/ctors/props/named lambdas/nested defs. No setter/alias/anonymous lambda callables. Async methods have kind `function` |
| 03 Call-site forms | partial | bare/attribute/ctor/`__call__`/kwargs/value refs. No arity, subscript callees or operator dunders |
| 04 Imports & module resolution | full | absolute/relative/aliased/star/re-export via ModuleIndex, external refusal. Comma imports break framework detection |
| 05 Type inference & receiver typing | partial | params, ctor assignment, self-field and annotations. Return types and local annotations mostly unused. Name heuristic yields false highs |
| 06 Inheritance, dispatch & overloading | partial | class records, super, typed inherited lookup, inheritance fan-out. Cross-file inherited `self.m()` fails. No overload narrowing |
| 07 Scoping & shadowing | partial | locals table + lexical nesting. Comprehension/loop-leak scoping wrong. No conditional-def handling |
| 08 Macros, preprocessor & codegen | n/a | no macros. Decorator/dataclass-generated members not synthesised |
| 09 FFI & cross-language links | partial | PyO3 (via same-named Python def), Kivy KV, protobuf servicers. No ctypes/cffi/Cython |
| 10 Frameworks & entry points | full | 49 rules (34 with matchers, 15 gap-only). Broadest rule set after Java |
| 11 Test harness entries | full | unittest + pytest harness nodes, test-class members rooted. conftest hooks are a declared gap |
| 12 Dead-code analysis | partial | visibility/exports/dunders/external-contract/reflection caps. Unreachable regions unused. Async-method cap bug |
| 13 Builtins & stdlib tables | partial | 1,180-name stdlib list + typing wrapper tables. No cast/callback/top-type entries |
| 14 Confidence & certainty | partial | anchored high paths well defined. Name-heuristic weak types still reach high intra-file |
| 15 Dynamic use & reflection | partial | getattr/setattr/hasattr/delattr/import_module literals. No eval/**import**/attrgetter |
| 16 Known gaps & limitations | n/a | 16 confirmed gaps listed above |
