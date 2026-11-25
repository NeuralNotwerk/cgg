//! Stdlib manifests — embedded at compile time.
//!
//! Each manifest is a newline-separated list of known stdlib symbol names
//! for a given language. Used by the external call classifier to identify
//! calls that target stdlib/framework code even when the callee name
//! happens to match a project-defined symbol.

use std::collections::HashSet;
use std::sync::OnceLock;

/// Perl: modules whose `use M 'Base', …` names the using package's base
/// classes (the core `parent` / `base` pragmas, and `Mojo::Base`, whose
/// first string argument is the base). Arguments starting with `-` are
/// flags (`-norequire`, `-base`, `-strict`).
pub const PERL_BASE_PRAGMAS: &[&str] = &["parent", "base", "Mojo::Base"];

/// Perl: OO keywords (Moose / Moo / Mouse) whose string arguments name
/// where the package's methods come from: `extends 'Base'` its base
/// classes, `with 'Role'` the roles composed into it.
pub const PERL_BASE_KEYWORDS: &[&str] = &["extends", "with"];

/// Kotlin standard-library scope functions whose lambda runs with the
/// receiver as `this`: `x.run { m() }`, `x.apply { m() }` (extension
/// form, receiver before the dot).
pub const KOTLIN_RECEIVER_SCOPE_EXTENSIONS: &[&str] = &["run", "apply"];

/// Kotlin scope functions taking the receiver as their first argument:
/// `with(x) { m() }`.
pub const KOTLIN_RECEIVER_SCOPE_FUNCTIONS: &[&str] = &["with"];

/// Cast operators spelled as a call rather than as syntax: the target
/// type is the call's type argument and the value is what is cast
/// (`static_cast<Foo*>(o)`, Scala `o.asInstanceOf[Foo]`). A receiver
/// written as one of these is an instance of the target type.
pub const CAST_INTRINSICS: &[(&str, &[&str])] = &[
    (
        "cpp",
        &[
            "static_cast",
            "dynamic_cast",
            "reinterpret_cast",
            "const_cast",
        ],
    ),
    ("scala", &["asInstanceOf"]),
];

/// Whether `name` is one of `lang`'s [`CAST_INTRINSICS`].
pub fn is_cast_intrinsic(lang: &str, name: &str) -> bool {
    CAST_INTRINSICS
        .iter()
        .any(|(l, names)| *l == lang && names.contains(&name))
}

/// Methods the language itself defines to construct an instance of the
/// class they are called on: Ruby's `Foo.new(…)` builds a `Foo`, an
/// Objective-C `[Foo alloc]` an uninitialised `Foo`. A receiver written
/// as one is an instance of that class.
pub const CONSTRUCTING_METHODS: &[(&str, &[&str])] =
    &[("ruby", &["new"]), ("objc", &["alloc"])];

/// Whether `name` is one of `lang`'s [`CONSTRUCTING_METHODS`].
pub fn is_constructing_method(lang: &str, name: &str) -> bool {
    CONSTRUCTING_METHODS
        .iter()
        .any(|(l, names)| *l == lang && names.contains(&name))
}

/// Standard-library modules and namespaces written as a call's qualifier
/// (`POSIX::floor`, `stats::median`, `clojure.string/join`) whose names
/// are not in the language's manifest. Read only by the external-call
/// classifier (`external::module_is_stdlib`), which tests the qualifier
/// and each of its `::`/`.`/`/`-separated prefixes against this table:
/// it never feeds name-based resolution, so a workspace function named
/// like a module (`base`, `stats`) is unaffected.
pub const STDLIB_MODULES: &[(&str, &[&str])] = &[
    (
        "perl",
        &[
            "Benchmark",
            "Carp",
            "Config",
            "Cwd",
            "Data::Dumper",
            "Digest::MD5",
            "Digest::SHA",
            "Encode",
            "Errno",
            "Exporter",
            "Fcntl",
            "File::Basename",
            "File::Compare",
            "File::Copy",
            "File::Find",
            "File::Glob",
            "File::Path",
            "File::Spec",
            "File::Spec::Functions",
            "File::Temp",
            "File::stat",
            "FindBin",
            "Getopt::Long",
            "Getopt::Std",
            "Hash::Util",
            "HTTP::Tiny",
            "I18N::Langinfo",
            "IO::File",
            "IO::Handle",
            "IO::Select",
            "IO::Socket",
            "IO::Socket::INET",
            "IPC::Cmd",
            "IPC::Open2",
            "IPC::Open3",
            "JSON::PP",
            "List::Util",
            "MIME::Base64",
            "Math::BigFloat",
            "Math::BigInt",
            "POSIX",
            "Pod::Usage",
            "Scalar::Util",
            "Socket",
            "Storable",
            "Sys::Hostname",
            "Term::ANSIColor",
            "Text::Abbrev",
            "Text::ParseWords",
            "Text::Wrap",
            "Tie::Hash",
            "Time::HiRes",
            "Time::Local",
            "Time::Piece",
            "UNIVERSAL",
            "Unicode::Normalize",
        ],
    ),
    (
        "r",
        &[
            "base",
            "compiler",
            "datasets",
            "grDevices",
            "graphics",
            "grid",
            "methods",
            "parallel",
            "splines",
            "stats",
            "stats4",
            "tcltk",
            "tools",
            "utils",
        ],
    ),
    (
        "clojure",
        &[
            "clojure.core",
            "clojure.core.protocols",
            "clojure.core.reducers",
            "clojure.core.server",
            "clojure.data",
            "clojure.datafy",
            "clojure.edn",
            "clojure.instant",
            "clojure.java.browse",
            "clojure.java.io",
            "clojure.java.javadoc",
            "clojure.java.shell",
            "clojure.main",
            "clojure.math",
            "clojure.pprint",
            "clojure.reflect",
            "clojure.repl",
            "clojure.set",
            "clojure.stacktrace",
            "clojure.string",
            "clojure.template",
            "clojure.test",
            "clojure.walk",
            "clojure.xml",
            "clojure.zip",
        ],
    ),
];

/// `lang`'s [`STDLIB_MODULES`], empty when it has none.
pub fn stdlib_modules(lang: &str) -> &'static [&'static str] {
    STDLIB_MODULES
        .iter()
        .find(|(l, _)| *l == lang)
        .map_or(&[], |(_, m)| m)
}

/// Scala: packages every file sees without an import, through the
/// implicit `import scala._` / `import java.lang._` — `math.ceil(x)` is
/// `scala.math.ceil`. A receiver headed by one of these that nothing in
/// the file or the tree binds is a call into the standard library.
pub const SCALA_IMPLICIT_PACKAGES: &[&str] = &[
    "scala",
    "java",
    "javax",
    "math",
    "collection",
    "concurrent",
    "util",
    "io",
    "sys",
    "reflect",
    "annotation",
    "runtime",
];

/// Rust `Result` / `Option` adapter methods whose names a workspace type
/// may also declare (a `Stats::ok`). A call of one through a receiver
/// the resolver could not type — overwhelmingly a chain, `s.parse().ok()`
/// — is the standard adapter, never fanned out by name. Kept out of the
/// general name list (`stdlib/rust.txt`) because that list also screens
/// bare calls, and a crate's own `err!` macro is a bare call.
pub const RUST_UNTYPED_ADAPTER_METHODS: &[&str] = &["ok", "err", "as_deref"];

/// Objective-C functions that make a selector from a string
/// (`NSSelectorFromString(@"tap:")`): a literal argument names a method.
pub const OBJC_SELECTOR_FROM_STRING: &[&str] = &["NSSelectorFromString"];

/// Clojure forms whose first argument is a binding vector of
/// `pattern init` pairs (`(let [x (f)] …)`): each pattern binds a local
/// over the form, and each init is a value.
pub const CLOJURE_BINDING_FORMS: &[&str] = &[
    "let",
    "let*",
    "loop",
    "loop*",
    "doseq",
    "for",
    "when-let",
    "if-let",
    "when-some",
    "if-some",
    "when-first",
    "dotimes",
];

/// Clojure forms whose binding vector rebinds vars (`(binding [*out* w]
/// …)`, `(with-redefs [f stub] …)`): each name is the var itself, never a
/// local, so `(f)` inside still calls the var `f`; each init is a value.
pub const CLOJURE_VAR_REBINDING_FORMS: &[&str] = &["binding", "with-redefs"];

/// Name suffixes of Clojure macros that, by the convention `with-redefs`
/// sets, redefine vars in their binding vector (a test library's
/// `with-dynamic-fn-redefs`): like `CLOJURE_VAR_REBINDING_FORMS`.
pub const CLOJURE_VAR_REBINDING_SUFFIXES: &[&str] = &["-redefs"];

/// Prefixes of Clojure macros whose first argument, when it is a vector,
/// binds locals in a shape cgg does not know (`with-open`, a library's
/// `with-temp`): every symbol directly in it is taken as bound and none as
/// a value (`CLOJURE_VAR_REBINDING_FORMS` excepted).
pub const CLOJURE_BINDING_FORM_PREFIXES: &[&str] = &["with-"];

/// Clojure forms that define a function: the parameter vector(s) after
/// the name (`fn` may name itself) bind locals over the form. The number
/// is how many leading arguments to skip before looking for parameters.
pub const CLOJURE_FN_FORMS: &[(&str, usize)] = &[
    ("fn", 0),
    ("fn*", 0),
    ("bound-fn", 0),
    ("defn", 1),
    ("defn-", 1),
    ("defmacro", 1),
    ("defmethod", 2),
];

/// Clojure forms whose body lists are method implementations
/// (`(m [this x] …)`), not calls: their parameter vectors bind locals.
/// `letfn`'s binding vector holds the same shape.
pub const CLOJURE_METHOD_BODY_FORMS: &[&str] = &[
    "letfn",
    "deftype",
    "defrecord",
    "reify",
    "proxy",
    "extend-type",
    "extend-protocol",
    "defprotocol",
    "definterface",
];

/// Clojure forms that declare methods (`(defprotocol P (m [this]))`): each
/// signature is a var of the namespace, implemented elsewhere.
pub const CLOJURE_PROTOCOL_DECL_FORMS: &[&str] = &["defprotocol", "definterface"];

/// Clojure forms whose method lists implement a protocol or interface
/// (`(deftype T [a] P (m [this] …))`): each is a method definition, not a
/// call.
pub const CLOJURE_METHOD_IMPL_FORMS: &[&str] = &[
    "deftype",
    "defrecord",
    "reify",
    "proxy",
    "extend-type",
    "extend-protocol",
];

/// Clojure forms whose second argument is a field vector (`(deftype T
/// [a b] …)`): the fields are locals in every method body.
pub const CLOJURE_TYPE_FIELD_FORMS: &[&str] = &["deftype", "defrecord"];

/// Clojure forms whose arguments are not values: quoted data, constants
/// (`case`), class names (`new`, `catch`), declarations and namespace
/// directives.
pub const CLOJURE_NON_VALUE_FORMS: &[&str] = &[
    "quote",
    "ns",
    "comment",
    "case",
    "catch",
    "new",
    ".",
    "..",
    "set!",
    "import",
    "require",
    "use",
    "refer",
    "declare",
    "in-ns",
    "defprotocol",
    "definterface",
    "deftype",
    "defrecord",
    "defstruct",
    "monitor-enter",
    "monitor-exit",
    "var",
];

/// Clojure forms that define a var from a value (`(def x f)`): the last
/// argument is a value.
pub const CLOJURE_DEF_FORMS: &[&str] = &["def", "defonce", "defmulti"];

/// The module of Erlang's auto-imported BIFs: `erlang:apply(M, F, A)` is
/// `apply(M, F, A)` (`ERLANG_MFA_CALLS` lists the unqualified spelling).
pub const ERLANG_BIF_MODULE: &str = "erlang";

/// Erlang functions that call `M:F(A…)` given the module, function and
/// argument list as separate arguments, keyed by the callee as written
/// (`erlang:` dropped from a BIF) and its argument count, with the
/// position of `M` (`F` and `A` follow): `spawn(M, F, A)`,
/// `spawn(Node, M, F, A)`, `timer:apply_after(T, M, F, A)`.
pub const ERLANG_MFA_CALLS: &[(&str, usize, usize)] = &[
    ("apply", 3, 0),
    ("spawn", 3, 0),
    ("spawn", 4, 1),
    ("spawn_link", 3, 0),
    ("spawn_link", 4, 1),
    ("spawn_monitor", 3, 0),
    ("spawn_monitor", 4, 1),
    ("spawn_opt", 4, 0),
    ("spawn_opt", 5, 1),
    ("proc_lib:spawn", 3, 0),
    ("proc_lib:spawn", 4, 1),
    ("proc_lib:spawn_link", 3, 0),
    ("proc_lib:spawn_link", 4, 1),
    ("proc_lib:spawn_opt", 4, 0),
    ("proc_lib:start", 3, 0),
    ("proc_lib:start", 4, 0),
    ("proc_lib:start", 5, 0),
    ("proc_lib:start_link", 3, 0),
    ("proc_lib:start_link", 4, 0),
    ("proc_lib:start_link", 5, 0),
    ("timer:apply_after", 4, 1),
    ("timer:apply_interval", 4, 1),
    ("timer:apply_repeatedly", 4, 1),
    ("rpc:call", 4, 1),
    ("rpc:call", 5, 1),
    ("rpc:cast", 4, 1),
    ("erpc:call", 4, 1),
    ("erpc:call", 5, 1),
    ("erpc:cast", 4, 1),
];

/// The Nix builtin that loads another file (`import ./lib.nix { … }`):
/// `inherit (import ./f.nix …) x;` is another name for that file's `x`
/// (WS27).
pub const NIX_IMPORT_BUILTIN: &str = "import";

/// R functions that bind a variable named by a string literal in the
/// calling function's environment (`assign("x", value)`), and the
/// arguments that send the binding elsewhere instead (WS27): with one of
/// those, it is not a local.
pub const R_ASSIGN_FUNCTIONS: &[&str] = &["assign"];
pub const R_ASSIGN_ELSEWHERE_ARGS: &[&str] = &["envir", "pos", "inherits"];

/// PHP core functions that take a callable, with the callable argument's
/// position: `array_map('f', $xs)`, `usort($xs, [$this, 'cmp'])`,
/// `spl_autoload_register([self::class, 'load'])`,
/// `Closure::fromCallable('f')` (a static call is keyed `Class::method`).
/// `call_user_func(_array)` and `is_callable` are left to the reflection
/// screen (`dynuse`): the first calls by name, the second only asks.
pub const PHP_CALLABLE_ARG_FUNCS: &[(&str, usize)] = &[
    ("array_map", 0),
    ("array_filter", 1),
    ("array_walk", 1),
    ("array_walk_recursive", 1),
    ("array_reduce", 1),
    ("usort", 1),
    ("uasort", 1),
    ("uksort", 1),
    ("iterator_apply", 1),
    ("preg_replace_callback", 1),
    ("spl_autoload_register", 0),
    ("register_shutdown_function", 0),
    ("register_tick_function", 0),
    ("set_error_handler", 0),
    ("set_exception_handler", 0),
    ("header_register_callback", 0),
    ("ob_start", 0),
    ("forward_static_call", 0),
    ("forward_static_call_array", 0),
    ("Closure::fromCallable", 0),
];

/// Ruby core methods whose symbol argument names a method of the
/// receiver (or of `self`), with that argument's position: `method(:m)`,
/// `obj.public_method(:m)`, `Klass.instance_method(:m)`, and the old name
/// of `alias_method :new, :old`. `send`/`public_send` call by name and
/// `respond_to?` only asks; the reflection screen (`dynuse`) reads those.
pub const RUBY_METHOD_NAMING_CALLS: &[(&str, usize)] = &[
    ("method", 0),
    ("public_method", 0),
    ("singleton_method", 0),
    ("instance_method", 0),
    ("public_instance_method", 0),
    ("alias_method", 1),
];

/// JavaScript/TypeScript `Function.prototype` methods that return the
/// function they are called on, bound: `this.m.bind(this)` passed or
/// stored as a value is a reference to `m`.
pub const JS_FUNCTION_VALUE_ADAPTERS: &[&str] = &["bind"];

/// The [`SCALA_IMPLICIT_PACKAGES`] that are roots in their own right;
/// every other entry is a child of `scala` (`math` is `scala.math`).
pub const SCALA_ROOT_PACKAGES: &[&str] = &["scala", "java", "javax"];

/// Type constructors that mean "a `T`, or nothing" (or "a `T`, with a
/// qualifier that changes nothing a method call sees"): an annotation
/// written as one of these types its value as the first type argument.
/// Containers (`list[T]`, `Option<T>` in Rust/Scala, Java's
/// `Optional<T>`) are deliberately absent: a method called on one runs
/// the container's own method, never `T`'s.
pub const MAYBE_TYPE_WRAPPERS: &[(&str, &[&str])] = &[
    (
        "python",
        &[
            "Optional",
            "ClassVar",
            "Final",
            "Annotated",
            "InitVar",
            "Required",
            "NotRequired",
            "ReadOnly",
        ],
    ),
    ("csharp", &["Nullable"]),
    ("swift", &["Optional", "ImplicitlyUnwrappedOptional"]),
    ("typescript", &["NonNullable", "Readonly"]),
];

/// Union type constructors written as a subscripted name rather than as
/// `A | B`: Python's `Union[A, B]`. Their arguments are union members.
pub const UNION_TYPE_CONSTRUCTORS: &[(&str, &[&str])] = &[("python", &["Union"])];

/// Union members that hold no object a method could be called on: `Foo |
/// None` is a `Foo` wherever a method is called on it. A language absent
/// here has no union types (or none written with `|`), and only a
/// `?` marker can make one of its annotations nullable.
pub const NULL_TYPE_MEMBERS: &[(&str, &[&str])] = &[
    ("python", &["None", "NoneType"]),
    ("typescript", &["null", "undefined", "void"]),
    ("javascript", &["null", "undefined", "void"]),
    ("php", &["null", "false", "void"]),
    ("scala", &["Null", "Nothing"]),
];

/// Callback types written as a named type constructor: the callback's
/// parameter types are its type arguments (`Action<Foo>`, C#; `void
/// Function(Foo)`, Dart), minus the last one when that is the result
/// (`Func<Foo, R>`). Scala's `Foo => R` is syntax and needs no entry.
pub const CALLBACK_TYPE_CONSTRUCTORS: &[(&str, &str, bool)] = &[
    ("csharp", "Action", false),
    ("csharp", "Func", true),
    ("dart", "Function", false),
];

/// Go's predeclared types: a parameter declared as one is no workspace
/// type, whatever a package of the tree declares under that name.
pub const GO_PREDECLARED_TYPES: &[&str] = &[
    "any",
    "bool",
    "byte",
    "comparable",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
];

/// Go's predeclared functions: `x := make(…)`, `p := new(T)` call no
/// function of the calling package (WS27).
pub const GO_BUILTIN_FUNCS: &[&str] = &[
    "append", "cap", "clear", "close", "complex", "copy", "delete", "imag", "len",
    "make", "max", "min", "new", "panic", "print", "println", "real", "recover",
];

/// Reflective APIs that enumerate a type's own members, per language
/// (WS28b): written on a type the code names — C# `typeof(T).GetMethods()`
/// and `typeof(T).GetTypeInfo().DeclaredMethods`, Java
/// `T.class.getDeclaredMethods()`, PHP `get_class_methods(T::class)` and
/// `(new ReflectionClass(T::class))->getMethods()`, Ruby
/// `T.instance_methods`, Python `inspect.getmembers(T)` — they hand every
/// method of `T` to code that may call it. Never on a value whose type
/// is not written (`x.GetType()`, `getClass()`).
pub const MEMBER_ENUMERATION_APIS: &[(&str, &[&str])] = &[
    (
        "csharp",
        &[
            "GetMethods",
            "GetMembers",
            "GetRuntimeMethods",
            "DeclaredMethods",
            "DeclaredMembers",
        ],
    ),
    ("java", &["getDeclaredMethods", "getMethods"]),
    ("php", &["get_class_methods", "getMethods"]),
    (
        "ruby",
        &[
            "instance_methods",
            "public_instance_methods",
            "private_instance_methods",
            "protected_instance_methods",
        ],
    ),
    ("python", &["getmembers"]),
];

/// Go standard-library interfaces a type satisfies by having the methods,
/// with no declaration naming them (WS31): each as written in an embed
/// (`io.Reader`), with its method set as (name, parameter count). The
/// standard library calls these methods (`fmt` calls `String`, `sort`
/// calls `Len`/`Less`/`Swap`, `database/sql` calls `Scan`), so a type
/// whose method set covers one implements a contract outside the tree;
/// an interface of the tree that embeds one has these methods too.
pub const GO_STDLIB_INTERFACES: &[(&str, &[(&str, usize)])] = &[
    ("error", &[("Error", 0)]),
    ("fmt.Stringer", &[("String", 0)]),
    ("fmt.GoStringer", &[("GoString", 0)]),
    ("fmt.Formatter", &[("Format", 2)]),
    ("io.Reader", &[("Read", 1)]),
    ("io.Writer", &[("Write", 1)]),
    ("io.Closer", &[("Close", 0)]),
    ("io.Seeker", &[("Seek", 2)]),
    ("io.ReaderAt", &[("ReadAt", 2)]),
    ("io.WriterAt", &[("WriteAt", 2)]),
    ("io.ReaderFrom", &[("ReadFrom", 1)]),
    ("io.WriterTo", &[("WriteTo", 1)]),
    ("io.ByteReader", &[("ReadByte", 0)]),
    ("io.ByteWriter", &[("WriteByte", 1)]),
    ("io.RuneReader", &[("ReadRune", 0)]),
    ("io.StringWriter", &[("WriteString", 1)]),
    ("io.ReadCloser", &[("Read", 1), ("Close", 0)]),
    ("io.WriteCloser", &[("Write", 1), ("Close", 0)]),
    ("io.ReadWriter", &[("Read", 1), ("Write", 1)]),
    (
        "io.ReadWriteCloser",
        &[("Read", 1), ("Write", 1), ("Close", 0)],
    ),
    ("sort.Interface", &[("Len", 0), ("Less", 2), ("Swap", 2)]),
    (
        "heap.Interface",
        &[
            ("Len", 0),
            ("Less", 2),
            ("Swap", 2),
            ("Push", 1),
            ("Pop", 0),
        ],
    ),
    ("flag.Value", &[("String", 0), ("Set", 1)]),
    ("sql.Scanner", &[("Scan", 1)]),
    ("driver.Valuer", &[("Value", 0)]),
    ("http.Handler", &[("ServeHTTP", 2)]),
    ("http.RoundTripper", &[("RoundTrip", 1)]),
    ("http.Flusher", &[("Flush", 0)]),
    ("json.Marshaler", &[("MarshalJSON", 0)]),
    ("json.Unmarshaler", &[("UnmarshalJSON", 1)]),
    ("encoding.TextMarshaler", &[("MarshalText", 0)]),
    ("encoding.TextUnmarshaler", &[("UnmarshalText", 1)]),
    ("encoding.BinaryMarshaler", &[("MarshalBinary", 0)]),
    ("encoding.BinaryUnmarshaler", &[("UnmarshalBinary", 1)]),
    ("xml.Marshaler", &[("MarshalXML", 2)]),
    ("xml.Unmarshaler", &[("UnmarshalXML", 2)]),
    (
        "context.Context",
        &[("Deadline", 0), ("Done", 0), ("Err", 0), ("Value", 1)],
    ),
    (
        "hash.Hash",
        &[
            ("Write", 1),
            ("Sum", 1),
            ("Reset", 0),
            ("Size", 0),
            ("BlockSize", 0),
        ],
    ),
    ("net.Addr", &[("Network", 0), ("String", 0)]),
    (
        "slog.Handler",
        &[
            ("Enabled", 2),
            ("Handle", 2),
            ("WithAttrs", 1),
            ("WithGroup", 1),
        ],
    ),
    ("slog.LogValuer", &[("LogValue", 0)]),
];

/// Types a function returns when it is asynchronous: a call on the
/// result reaches the awaitable itself, or (awaited) the type it wraps,
/// so a declared `Task<Foo>` says nothing certain about the receiver.
pub const AWAITABLE_TYPES: &[(&str, &[&str])] = &[
    ("csharp", &["Task", "ValueTask", "IAsyncEnumerable"]),
    ("dart", &["Future", "FutureOr", "Stream"]),
    ("typescript", &["Promise", "PromiseLike"]),
    ("javascript", &["Promise"]),
    ("cpp", &["future", "shared_future", "task"]),
    ("java", &["CompletableFuture", "CompletionStage", "Future"]),
];

/// Whether `name` is one of `lang`'s [`AWAITABLE_TYPES`].
pub fn is_awaitable_type(lang: &str, name: &str) -> bool {
    lang_table(AWAITABLE_TYPES, lang).contains(&name)
}

/// Types every value has: a parameter declared as one says nothing about
/// which class a method called on it belongs to.
pub const TOP_TYPES: &[(&str, &[&str])] = &[
    ("scala", &["Any", "AnyRef", "AnyVal"]),
    ("csharp", &["Object", "Dynamic"]),
    ("dart", &["Object", "Dynamic"]),
    ("java", &["Object"]),
    ("groovy", &["Object"]),
    // `Unit` and `Nothing` name no class a call reaches either.
    ("kotlin", &["Any", "Unit", "Nothing"]),
];

/// Whether `name` is one of `lang`'s [`TOP_TYPES`].
pub fn is_top_type(lang: &str, name: &str) -> bool {
    lang_table(TOP_TYPES, lang).contains(&name)
}

fn lang_table<'a>(table: &'a [(&str, &'a [&'a str])], lang: &str) -> &'a [&'a str] {
    table
        .iter()
        .find(|(l, _)| *l == lang)
        .map_or(&[], |(_, names)| names)
}

/// Whether `head` is one of `lang`'s [`MAYBE_TYPE_WRAPPERS`].
pub fn is_maybe_type_wrapper(lang: &str, head: &str) -> bool {
    lang_table(MAYBE_TYPE_WRAPPERS, lang).contains(&head)
}

/// Whether `head` is one of `lang`'s [`UNION_TYPE_CONSTRUCTORS`].
pub fn is_union_type_constructor(lang: &str, head: &str) -> bool {
    lang_table(UNION_TYPE_CONSTRUCTORS, lang).contains(&head)
}

/// Whether `member` is one of `lang`'s [`NULL_TYPE_MEMBERS`].
pub fn is_null_type_member(lang: &str, member: &str) -> bool {
    lang_table(NULL_TYPE_MEMBERS, lang).contains(&member)
}

static RUST: &str = include_str!("stdlib/rust.txt");
/// CMake's built-in commands (`cmake --help-command-list`, 3.28). A
/// project `function(message)` overriding one stays reachable in its
/// own file but is never matched by name from another.
static CMAKE: &str = include_str!("stdlib/cmake.txt");
static PYTHON: &str = include_str!("stdlib/python.txt");
static GO: &str = include_str!("stdlib/go.txt");
static JAVASCRIPT: &str = include_str!("stdlib/javascript.txt");
static TYPESCRIPT: &str = include_str!("stdlib/typescript.txt");
static JAVA: &str = include_str!("stdlib/java.txt");
static KOTLIN: &str = include_str!("stdlib/kotlin.txt");
static C: &str = include_str!("stdlib/c.txt");
static CPP: &str = include_str!("stdlib/cpp.txt");
static CSHARP: &str = include_str!("stdlib/csharp.txt");
static BASH: &str = include_str!("stdlib/bash.txt");
static RUBY: &str = include_str!("stdlib/ruby.txt");
static PHP: &str = include_str!("stdlib/php.txt");
static OBJC: &str = include_str!("stdlib/objc.txt");
static R: &str = include_str!("stdlib/r.txt");
static SWIFT: &str = include_str!("stdlib/swift.txt");
static LUA: &str = include_str!("stdlib/lua.txt");
static DART: &str = include_str!("stdlib/dart.txt");
static SCALA: &str = include_str!("stdlib/scala.txt");
static HCL: &str = include_str!("stdlib/hcl.txt");
static ZIG: &str = include_str!("stdlib/zig.txt");
static GROOVY: &str = include_str!("stdlib/groovy.txt");
static JULIA: &str = include_str!("stdlib/julia.txt");
static PERL: &str = include_str!("stdlib/perl.txt");
static ELIXIR: &str = include_str!("stdlib/elixir.txt");
static ERLANG: &str = include_str!("stdlib/erlang.txt");
static FORTRAN: &str = include_str!("stdlib/fortran.txt");
static CLOJURE: &str = include_str!("stdlib/clojure.txt");
static HASKELL: &str = include_str!("stdlib/haskell.txt");
static OCAML: &str = include_str!("stdlib/ocaml.txt");

fn parse(src: &str) -> HashSet<&str> {
    src.lines().filter(|l| !l.is_empty()).collect()
}

/// Get the stdlib symbol set for a language id.
/// Returns None for unrecognized languages.
pub fn stdlib_names(lang: &str) -> Option<&'static HashSet<&'static str>> {
    static CACHE: OnceLock<Vec<(&'static str, HashSet<&'static str>)>> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        vec![
            ("rust", parse(RUST)),
            ("python", parse(PYTHON)),
            ("go", parse(GO)),
            ("javascript", parse(JAVASCRIPT)),
            ("typescript", parse(TYPESCRIPT)),
            ("java", parse(JAVA)),
            ("kotlin", parse(KOTLIN)),
            ("c", parse(C)),
            ("cpp", parse(CPP)),
            ("csharp", parse(CSHARP)),
            ("bash", parse(BASH)),
            ("ruby", parse(RUBY)),
            ("php", parse(PHP)),
            ("objc", parse(OBJC)),
            ("r", parse(R)),
            ("swift", parse(SWIFT)),
            ("lua", parse(LUA)),
            ("dart", parse(DART)),
            ("scala", parse(SCALA)),
            ("hcl", parse(HCL)),
            ("zig", parse(ZIG)),
            ("groovy", parse(GROOVY)),
            ("julia", parse(JULIA)),
            ("perl", parse(PERL)),
            ("elixir", parse(ELIXIR)),
            ("erlang", parse(ERLANG)),
            ("fortran", parse(FORTRAN)),
            ("clojure", parse(CLOJURE)),
            ("haskell", parse(HASKELL)),
            ("ocaml", parse(OCAML)),
            ("cmake", parse(CMAKE)),
        ]
    });
    all.iter().find(|(id, _)| *id == lang).map(|(_, set)| set)
}

/// How a function-defining macro from outside the tree names the function
/// its body defines (`MACRO(args) { body }`). See
/// [`C_FN_DEFINING_MACROS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnMacroNaming {
    /// The first argument is the function's name: `BPF_KPROBE(name, …)`.
    First,
    /// `MACRO(Class, method)`: the method `method` of `Class`.
    OwnerMethod,
    /// `MACRO(suite, case)`: the test `case`, qualified `suite::case`.
    SuiteCase,
    /// `MACRO("description", …)`: no identifier names it; the callable is
    /// `MACRO_at_<line>` and the description rides in the attribute.
    Described,
}

/// Function-defining macros of C libraries and test harnesses whose
/// `#define` is outside the tree (`c_macro` expands in-tree ones). The
/// grammar reads `PHP_METHOD(Redis, get) { … }` as a call followed by a
/// stray block, or as a function named for the macro, so without this
/// table the body belongs to no callable (or to one named `PHP_METHOD`).
/// The callable carries the invocation text (`ZTEST(suite, case)`) as an
/// attribute, so a framework rule can match the macro by name.
///
/// The third field is whether C++ files read it too: `TEST` and `Test` in
/// C++ are gtest's (named for the macro, matched by the `gtest` rule) or
/// an in-class constructor, so they stay C-only; Catch2 and doctest share
/// the string-described `TEST_CASE` shape.
pub const C_FN_DEFINING_MACROS: &[(&str, FnMacroNaming, bool)] = &[
    // PHP / Zend extensions.
    ("PHP_METHOD", FnMacroNaming::OwnerMethod, true),
    ("ZEND_METHOD", FnMacroNaming::OwnerMethod, true),
    // libbpf program macros (bpf_tracing.h, bpf_core_read.h, usdt.bpf.h).
    ("BPF_PROG", FnMacroNaming::First, false),
    ("BPF_PROG2", FnMacroNaming::First, false),
    ("BPF_KPROBE", FnMacroNaming::First, false),
    ("BPF_KRETPROBE", FnMacroNaming::First, false),
    ("BPF_KSYSCALL", FnMacroNaming::First, false),
    ("BPF_KPROBE_SYSCALL", FnMacroNaming::First, false),
    ("BPF_UPROBE", FnMacroNaming::First, false),
    ("BPF_URETPROBE", FnMacroNaming::First, false),
    ("BPF_USDT", FnMacroNaming::First, false),
    // Zephyr ztest.
    ("ZTEST", FnMacroNaming::SuiteCase, true),
    ("ZTEST_F", FnMacroNaming::SuiteCase, true),
    ("ZTEST_USER", FnMacroNaming::SuiteCase, true),
    ("ZTEST_USER_F", FnMacroNaming::SuiteCase, true),
    // Criterion.
    ("Test", FnMacroNaming::SuiteCase, false),
    ("ParameterizedTest", FnMacroNaming::SuiteCase, false),
    // Unity Fixture.
    ("TEST", FnMacroNaming::SuiteCase, false),
    ("IGNORE_TEST", FnMacroNaming::SuiteCase, false),
    // ESP-IDF / Unity test cases, Catch2 and doctest.
    ("TEST_CASE", FnMacroNaming::Described, true),
    ("TEST_CASE_MULTIPLE_STAGES", FnMacroNaming::Described, false),
    (
        "TEST_CASE_MULTIPLE_DEVICES",
        FnMacroNaming::Described,
        false,
    ),
    ("SCENARIO", FnMacroNaming::Described, true),
    ("TEST_CASE_METHOD", FnMacroNaming::Described, true),
];

/// The naming rule of a function-defining macro, if `name` is one in
/// `language` (`c`, or `cpp`/`objc` reading the C++-safe entries).
pub fn fn_defining_macro(name: &str, cpp: bool) -> Option<FnMacroNaming> {
    C_FN_DEFINING_MACROS
        .iter()
        .find(|(m, _, in_cpp)| *m == name && (!cpp || *in_cpp))
        .map(|(_, n, _)| *n)
}

/// C and C++ type keywords and the standard library's type names: never a
/// value, though a macro argument (`va_arg(ap, int)`, `luaM_newvector(L,
/// n, int)`) puts one where a value stands (WS27).
pub const C_TYPE_NAMES: &[&str] = &[
    "auto",
    "bool",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "const",
    "double",
    "float",
    "int",
    "long",
    "short",
    "signed",
    "unsigned",
    "void",
    "volatile",
    "wchar_t",
    "_Bool",
    "_Complex",
    "size_t",
    "ssize_t",
    "ptrdiff_t",
    "intptr_t",
    "uintptr_t",
    "intmax_t",
    "uintmax_t",
    "off_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "va_list",
    "FILE",
    "nullptr_t",
];

// ---------------------------------------------------------------------
// `--externals` boundary tables.
//
// Read only by `cgg_resolve::externals` and the `--externals` extraction
// walk (`cgg_lang::externals`). Nothing here changes resolution.
// ---------------------------------------------------------------------

/// Module-path roots that are the language's standard library, as an
/// import or a written qualifier names them. A module equal to a root, or
/// starting with a root followed by a separator, is stdlib. Complements
/// the flat `stdlib/*.txt` vocabularies, which list call names.
pub const STDLIB_ROOTS: &[(&str, &[&str])] = &[
    ("rust", &["std", "core", "alloc", "proc_macro", "test"]),
    ("java", &["java", "javax", "jdk", "sun", "com.sun"]),
    ("kotlin", &["kotlin", "java", "javax"]),
    ("scala", &["scala", "java", "javax"]),
    (
        "groovy",
        &["groovy", "java", "javax", "org.codehaus.groovy"],
    ),
    (
        "clojure",
        &[
            "clojure.core",
            "clojure.string",
            "clojure.set",
            "clojure.java",
            "clojure.walk",
            "clojure.edn",
            "clojure.pprint",
            "clojure.test",
            "clojure.zip",
            "clojure.data",
            "clojure.repl",
            "java",
            "javax",
        ],
    ),
    ("csharp", &["System", "Microsoft.CSharp", "Microsoft.Win32"]),
    ("fsharp", &["System", "FSharp.Core", "Microsoft.FSharp"]),
    ("powershell", &["Microsoft.PowerShell", "System"]),
    ("dart", &["dart:"]),
    ("zig", &["std", "builtin", "root"]),
    ("lean", &["Init", "Lean", "Std"]),
    ("nix", &["builtins", "<nixpkgs/lib>"]),
    (
        "elixir",
        &[
            "Kernel",
            "Enum",
            "String",
            "Map",
            "List",
            "Keyword",
            "Process",
            "GenServer",
            "Supervisor",
            "Agent",
            "Task",
            "Logger",
            "File",
            "Path",
            "IO",
            "System",
            "Application",
            "Code",
            "Module",
            "Registry",
            "Stream",
            "Regex",
            "DateTime",
            "Date",
            "Time",
            "NaiveDateTime",
            "URI",
            "Base",
            "Integer",
            "Float",
            "Tuple",
            "Atom",
            "Access",
            "Exception",
            "ExUnit",
            "Mix",
            "EEx",
            "Port",
            "Node",
            "MapSet",
            "Range",
            "Inspect",
            "Protocol",
            "Calendar",
            "Version",
            "Bitwise",
        ],
    ),
    // `base` (and the GHC boot libraries every install ships), module by
    // module: `Data.Aeson` is a package, `Data.List` is not.
    (
        "haskell",
        &[
            "Prelude",
            "Data.List",
            "Data.Maybe",
            "Data.Char",
            "Data.Either",
            "Data.Function",
            "Data.Foldable",
            "Data.Traversable",
            "Data.IORef",
            "Data.STRef",
            "Data.Monoid",
            "Data.Semigroup",
            "Data.Ord",
            "Data.Bits",
            "Data.Word",
            "Data.Int",
            "Data.Ratio",
            "Data.Complex",
            "Data.Functor",
            "Data.Kind",
            "Data.Proxy",
            "Data.Void",
            "Data.Coerce",
            "Data.Typeable",
            "Data.Dynamic",
            "Data.String",
            "Data.Tuple",
            "Data.Bool",
            "Data.Eq",
            "Data.Fixed",
            "Data.Unique",
            "Data.Version",
            "Data.List.NonEmpty",
            "Data.Bifunctor",
            "Data.Map",
            "Data.Set",
            "Data.IntMap",
            "Data.IntSet",
            "Data.Sequence",
            "Data.Tree",
            "Data.Graph",
            "Data.ByteString",
            "Data.Text",
            "Data.Array",
            "Control.Monad",
            "Control.Applicative",
            "Control.Exception",
            "Control.Concurrent",
            "Control.Arrow",
            "Control.Category",
            "Control.DeepSeq",
            "System.IO",
            "System.Exit",
            "System.Environment",
            "System.Info",
            "System.Mem",
            "System.Timeout",
            "System.CPUTime",
            "System.Directory",
            "System.FilePath",
            "System.Process",
            "System.Posix",
            "Text.Printf",
            "Text.Read",
            "Text.Show",
            "Foreign",
            "GHC",
            "Numeric",
            "Debug.Trace",
            "Unsafe.Coerce",
            "Type.Reflection",
        ],
    ),
    (
        "ocaml",
        &[
            "Stdlib",
            "List",
            "Array",
            "String",
            "Bytes",
            "Char",
            "Printf",
            "Format",
            "Hashtbl",
            "Map",
            "Set",
            "Buffer",
            "Option",
            "Result",
            "Seq",
            "Fun",
            "Int",
            "Float",
            "Sys",
            "Filename",
            "Printexc",
            "Lazy",
            "Queue",
            "Stack",
            "Random",
            "Lexing",
            "Parsing",
            "Scanf",
            "Gc",
            "Arg",
            "In_channel",
            "Out_channel",
            "Unix",
            "Str",
            "Thread",
            "Mutex",
            "Condition",
            "Domain",
            "Atomic",
            "Effect",
        ],
    ),
    (
        "julia",
        &[
            "Base",
            "Core",
            "LinearAlgebra",
            "Statistics",
            "Random",
            "Dates",
            "Printf",
            "Test",
            "Distributed",
            "SparseArrays",
            "Serialization",
            "Sockets",
            "Logging",
            "Markdown",
            "InteractiveUtils",
            "Pkg",
            "SHA",
            "UUIDs",
            "Unicode",
            "DelimitedFiles",
            "Mmap",
            "Libdl",
            "Profile",
            "REPL",
            "TOML",
            "Downloads",
            "Tar",
            "FileWatching",
        ],
    ),
    (
        "r",
        &[
            "base",
            "stats",
            "utils",
            "methods",
            "graphics",
            "grDevices",
            "datasets",
            "tools",
            "parallel",
            "grid",
            "splines",
            "stats4",
            "tcltk",
            "compiler",
        ],
    ),
    (
        "lua",
        &[
            "string",
            "table",
            "os",
            "io",
            "math",
            "coroutine",
            "debug",
            "package",
            "utf8",
            "bit32",
            "jit",
        ],
    ),
    (
        "swift",
        &[
            "Swift",
            "Foundation",
            "Dispatch",
            "XCTest",
            "os",
            "Darwin",
            "Glibc",
            "Combine",
            "Observation",
            "Testing",
        ],
    ),
    ("objc", &["Foundation"]),
    (
        "fortran",
        &[
            "iso_c_binding",
            "iso_fortran_env",
            "ieee_arithmetic",
            "ieee_exceptions",
            "ieee_features",
            "omp_lib",
        ],
    ),
    ("vhdl", &["ieee", "std"]),
    ("proto", &["google/protobuf/"]),
    ("smithy", &["smithy.api"]),
    ("starlark", &["@bazel_tools"]),
    (
        "erlang",
        &[
            "kernel",
            "stdlib",
            "erts",
            "lists",
            "maps",
            "io",
            "io_lib",
            "gen_server",
            "gen_statem",
            "gen_event",
            "supervisor",
            "application",
            "ets",
            "dets",
            "mnesia",
            "file",
            "filename",
            "os",
            "erlang",
            "string",
            "binary",
            "proplists",
            "dict",
            "orddict",
            "sets",
            "ordsets",
            "gb_trees",
            "gb_sets",
            "queue",
            "timer",
            "calendar",
            "crypto",
            "ssl",
            "inets",
            "httpc",
            "logger",
            "rand",
            "unicode",
            "re",
            "math",
            "code",
            "init",
            "net_kernel",
            "rpc",
            "global",
            "pg",
            "persistent_term",
            "atomics",
            "counters",
            "zlib",
            "base64",
            "uri_string",
            "eunit",
            "ct",
            "error_logger",
            "lager",
        ],
    ),
    ("javascript", &["node:"]),
    ("typescript", &["node:"]),
];

/// The library a stdlib call with no import evidence is attributed to.
pub const STDLIB_LIBRARY_NAMES: &[(&str, &str)] = &[
    ("rust", "std"),
    ("python", "builtins"),
    ("kivy", "kivy-builtins"),
    ("javascript", "ecmascript"),
    ("typescript", "ecmascript"),
    ("go", "builtin"),
    ("java", "java.lang"),
    ("kotlin", "kotlin"),
    ("scala", "scala"),
    ("groovy", "groovy"),
    ("c", "libc"),
    ("cpp", "std"),
    ("objc", "Foundation"),
    ("csharp", "System"),
    ("fsharp", "FSharp.Core"),
    ("bash", "shell"),
    ("powershell", "Microsoft.PowerShell"),
    ("ruby", "core"),
    ("swift", "Swift"),
    ("lua", "lua"),
    ("php", "php"),
    ("dart", "dart:core"),
    ("hcl", "terraform"),
    ("zig", "std"),
    ("r", "base"),
    ("julia", "Base"),
    ("perl", "CORE"),
    ("elixir", "Kernel"),
    ("erlang", "erlang"),
    ("fortran", "intrinsic"),
    ("clojure", "clojure.core"),
    ("haskell", "Prelude"),
    ("ocaml", "Stdlib"),
    ("solidity", "solidity"),
    ("starlark", "starlark"),
    ("cmake", "cmake"),
    ("nix", "builtins"),
    ("verilog", "system-tasks"),
    ("vhdl", "std"),
    ("lean", "Init"),
    ("asm", "asm"),
    ("smithy", "smithy.api"),
    ("proto", "google.protobuf"),
    ("graphql", "graphql"),
    ("openapi", "openapi"),
    ("asyncapi", "asyncapi"),
];

/// Calls that run an external program, per language: a qualified callee
/// suffix (`subprocess.run`, `exec.Command`) or a bare name. The program
/// is the call's first string literal argument (its first word).
pub const PROCESS_SPAWN_APIS: &[(&str, &[&str])] = &[
    ("rust", &["Command::new"]),
    (
        "python",
        &[
            "subprocess.run",
            "subprocess.call",
            "subprocess.check_call",
            "subprocess.check_output",
            "subprocess.Popen",
            "os.system",
            "os.popen",
            "os.execv",
            "os.execvp",
            "os.execve",
            "os.execl",
            "os.execlp",
            "os.spawnv",
            "os.spawnl",
            "os.posix_spawn",
            "asyncio.create_subprocess_exec",
            "asyncio.create_subprocess_shell",
            "pty.spawn",
        ],
    ),
    (
        "javascript",
        &[
            "child_process.exec",
            "child_process.execSync",
            "child_process.execFile",
            "child_process.execFileSync",
            "child_process.spawn",
            "child_process.spawnSync",
            "child_process.fork",
            "execa",
            "execSync",
            "execFileSync",
            "spawnSync",
            "execFile",
            "spawn",
            "exec",
            "Deno.Command",
            "Bun.spawn",
            "Bun.spawnSync",
        ],
    ),
    (
        "typescript",
        &[
            "child_process.exec",
            "child_process.execSync",
            "child_process.execFile",
            "child_process.execFileSync",
            "child_process.spawn",
            "child_process.spawnSync",
            "child_process.fork",
            "execa",
            "execSync",
            "execFileSync",
            "spawnSync",
            "execFile",
            "spawn",
            "exec",
            "Deno.Command",
            "Bun.spawn",
            "Bun.spawnSync",
        ],
    ),
    (
        "go",
        &[
            "exec.Command",
            "exec.CommandContext",
            "syscall.Exec",
            "os.StartProcess",
        ],
    ),
    (
        "java",
        &["Runtime.exec", "getRuntime().exec", "ProcessBuilder"],
    ),
    (
        "kotlin",
        &["Runtime.exec", "getRuntime().exec", "ProcessBuilder"],
    ),
    (
        "scala",
        &[
            "Runtime.exec",
            "getRuntime().exec",
            "ProcessBuilder",
            "Process",
        ],
    ),
    (
        "groovy",
        &[
            "Runtime.exec",
            "getRuntime().exec",
            "ProcessBuilder",
            "execute",
        ],
    ),
    (
        "clojure",
        &["sh", "clojure.java.shell/sh", "ProcessBuilder."],
    ),
    ("csharp", &["Process.Start", "ProcessStartInfo"]),
    ("fsharp", &["Process.Start", "ProcessStartInfo"]),
    ("powershell", &["Start-Process", "Invoke-Expression", "&"]),
    (
        "c",
        &[
            "system",
            "popen",
            "execl",
            "execlp",
            "execle",
            "execv",
            "execvp",
            "execvpe",
            "execve",
            "posix_spawn",
            "posix_spawnp",
            "CreateProcessA",
            "CreateProcessW",
            "ShellExecuteA",
            "ShellExecuteW",
            "ShellExecuteExW",
            "WinExec",
            "_popen",
            "_wpopen",
            "_spawnl",
            "_spawnv",
        ],
    ),
    (
        "cpp",
        &[
            "system",
            "popen",
            "execl",
            "execlp",
            "execle",
            "execv",
            "execvp",
            "execvpe",
            "execve",
            "posix_spawn",
            "posix_spawnp",
            "CreateProcessA",
            "CreateProcessW",
            "ShellExecuteA",
            "ShellExecuteW",
            "ShellExecuteExW",
            "WinExec",
            "_popen",
            "_wpopen",
            "boost::process::system",
            "boost::process::child",
            "QProcess::start",
            "QProcess::execute",
        ],
    ),
    (
        "objc",
        &[
            "system",
            "popen",
            "execv",
            "execvp",
            "posix_spawn",
            "NSTask.launchedTaskWithLaunchPath",
            "launchedTaskWithLaunchPath",
        ],
    ),
    (
        "swift",
        &["Process.run", "Process", "NSTask", "posix_spawn", "system"],
    ),
    (
        "ruby",
        &[
            "system",
            "spawn",
            "exec",
            "Open3.capture2",
            "Open3.capture2e",
            "Open3.capture3",
            "Open3.popen3",
            "Open3.popen2",
            "IO.popen",
            "Process.spawn",
            "PTY.spawn",
            "Kernel.system",
            "Kernel.spawn",
        ],
    ),
    (
        "php",
        &[
            "exec",
            "shell_exec",
            "system",
            "passthru",
            "proc_open",
            "popen",
            "pcntl_exec",
        ],
    ),
    (
        "perl",
        &[
            "system",
            "exec",
            "qx",
            "readpipe",
            "IPC::Open3::open3",
            "IPC::Open2::open2",
            "open3",
            "open2",
        ],
    ),
    ("lua", &["os.execute", "io.popen"]),
    (
        "r",
        &[
            "system",
            "system2",
            "shell",
            "processx::run",
            "processx::process",
        ],
    ),
    ("julia", &["run", "pipeline", "open", "read"]),
    ("dart", &["Process.run", "Process.start", "Process.runSync"]),
    (
        "zig",
        &[
            "std.process.Child.init",
            "std.process.Child.run",
            "Child.init",
            "Child.run",
            "std.process.execv",
            "std.process.execve",
        ],
    ),
    (
        "elixir",
        &["System.cmd", "System.shell", ":os.cmd", "Port.open"],
    ),
    ("erlang", &["os:cmd", "open_port", "erlang:open_port"]),
    (
        "haskell",
        &[
            "System.Process.callCommand",
            "callCommand",
            "callProcess",
            "readProcess",
            "readProcessWithExitCode",
            "createProcess",
            "spawnProcess",
            "spawnCommand",
            "system",
            "rawSystem",
            "proc",
            "shell",
        ],
    ),
    (
        "ocaml",
        &[
            "Unix.system",
            "Unix.create_process",
            "Unix.open_process_in",
            "Unix.open_process",
            "Unix.execv",
            "Unix.execvp",
            "Sys.command",
        ],
    ),
    ("fortran", &["execute_command_line", "system"]),
    (
        "cmake",
        &["execute_process", "add_custom_command", "add_custom_target"],
    ),
    (
        "nix",
        &[
            "runCommand",
            "pkgs.runCommand",
            "runCommandLocal",
            "writeShellScript",
            "writeShellScriptBin",
        ],
    ),
    (
        "starlark",
        &[
            "genrule",
            "ctx.actions.run",
            "ctx.actions.run_shell",
            "repository_ctx.execute",
            "rctx.execute",
        ],
    ),
    (
        "lean",
        &["IO.Process.spawn", "IO.Process.output", "IO.Process.run"],
    ),
];

/// Calls that load a native library at run time, per language. The
/// library is the first string literal argument.
pub const LIBRARY_LOAD_APIS: &[(&str, &[&str])] = &[
    (
        "rust",
        &["libloading::Library::new", "Library::new", "dlopen"],
    ),
    (
        "python",
        &[
            "ctypes.CDLL",
            "ctypes.cdll.LoadLibrary",
            "ctypes.WinDLL",
            "ctypes.windll.LoadLibrary",
            "ctypes.OleDLL",
            "ctypes.PyDLL",
            "CDLL",
            "WinDLL",
            "cdll.LoadLibrary",
            "windll.LoadLibrary",
            "ffi.dlopen",
            "cffi.FFI",
            "ctypes.util.find_library",
        ],
    ),
    (
        "javascript",
        &[
            "ffi.Library",
            "koffi.load",
            "process.dlopen",
            "Deno.dlopen",
            "dlopen",
        ],
    ),
    (
        "typescript",
        &[
            "ffi.Library",
            "koffi.load",
            "process.dlopen",
            "Deno.dlopen",
            "dlopen",
        ],
    ),
    (
        "go",
        &[
            "syscall.LoadLibrary",
            "syscall.LoadDLL",
            "syscall.NewLazyDLL",
            "windows.LoadLibrary",
            "windows.NewLazySystemDLL",
            "windows.NewLazyDLL",
            "plugin.Open",
            "purego.Dlopen",
        ],
    ),
    (
        "java",
        &[
            "System.loadLibrary",
            "System.load",
            "Runtime.loadLibrary",
            "Native.load",
            "Native.loadLibrary",
            "NativeLibrary.getInstance",
            "SymbolLookup.libraryLookup",
        ],
    ),
    (
        "kotlin",
        &[
            "System.loadLibrary",
            "System.load",
            "Native.load",
            "Native.loadLibrary",
        ],
    ),
    (
        "scala",
        &["System.loadLibrary", "System.load", "Native.load"],
    ),
    (
        "groovy",
        &["System.loadLibrary", "System.load", "Native.load"],
    ),
    (
        "clojure",
        &[
            "System/loadLibrary",
            "System/load",
            "clojure.lang.RT/loadLibrary",
        ],
    ),
    (
        "csharp",
        &[
            "NativeLibrary.Load",
            "NativeLibrary.TryLoad",
            "LoadLibrary",
            "LoadLibraryEx",
            "Assembly.LoadFrom",
            "Assembly.LoadFile",
            "Assembly.Load",
        ],
    ),
    (
        "fsharp",
        &[
            "NativeLibrary.Load",
            "NativeLibrary.TryLoad",
            "Assembly.LoadFrom",
        ],
    ),
    (
        "c",
        &[
            "dlopen",
            "dlmopen",
            "LoadLibraryA",
            "LoadLibraryW",
            "LoadLibraryExA",
            "LoadLibraryExW",
            "LoadLibrary",
            "GetModuleHandleA",
            "GetModuleHandleW",
        ],
    ),
    (
        "cpp",
        &[
            "dlopen",
            "dlmopen",
            "LoadLibraryA",
            "LoadLibraryW",
            "LoadLibraryExA",
            "LoadLibraryExW",
            "LoadLibrary",
            "GetModuleHandleA",
            "GetModuleHandleW",
            "QLibrary::load",
            "boost::dll::shared_library",
        ],
    ),
    (
        "objc",
        &["dlopen", "NSBundle.bundleWithPath", "bundleWithPath"],
    ),
    ("swift", &["dlopen", "Bundle.init"]),
    (
        "ruby",
        &[
            "ffi_lib",
            "Fiddle.dlopen",
            "Fiddle::Handle.new",
            "DL.dlopen",
        ],
    ),
    ("php", &["FFI::cdef", "FFI::load", "FFI::scope", "dl"]),
    (
        "perl",
        &[
            "DynaLoader::dl_load_file",
            "XSLoader::load",
            "bootstrap",
            "FFI::Platypus.new",
            "dl_load_file",
        ],
    ),
    ("lua", &["ffi.load", "package.loadlib"]),
    ("r", &["dyn.load", "library.dynam", "useDynLib"]),
    ("julia", &["dlopen", "Libdl.dlopen"]),
    (
        "dart",
        &[
            "DynamicLibrary.open",
            "DynamicLibrary.process",
            "DynamicLibrary.executable",
        ],
    ),
    ("zig", &["std.DynLib.open", "DynLib.open"]),
    ("elixir", &[":erlang.load_nif"]),
    ("erlang", &["erlang:load_nif", "load_nif"]),
    ("haskell", &["dlopen", "System.Posix.DynamicLinker.dlopen"]),
    ("ocaml", &["Dl.dlopen", "Dynlink.loadfile"]),
    ("lean", &["Lean.loadDynlib"]),
    (
        "powershell",
        &[
            "Add-Type",
            "[System.Runtime.InteropServices.NativeLibrary]::Load",
        ],
    ),
];

/// OS interfaces reached as a module or package, per language: a call
/// attributed to one of these is `system`, not `stdlib`/`third-party`.
pub const OS_BINDING_MODULES: &[(&str, &[&str])] = &[
    (
        "rust",
        &[
            "libc",
            "nix",
            "windows_sys",
            "windows",
            "winapi",
            "rustix",
            "syscalls",
        ],
    ),
    (
        "go",
        &[
            "syscall",
            "golang.org/x/sys/unix",
            "golang.org/x/sys/windows",
            "golang.org/x/sys",
        ],
    ),
    (
        "python",
        &[
            "posix",
            "nt",
            "fcntl",
            "termios",
            "resource",
            "msvcrt",
            "winreg",
            "_winapi",
            "mmap",
            "pwd",
            "grp",
            "ctypes.windll",
        ],
    ),
    ("javascript", &["node:os", "os"]),
    ("typescript", &["node:os", "os"]),
    (
        "zig",
        &["std.os", "std.posix", "std.os.linux", "std.os.windows"],
    ),
    ("swift", &["Darwin", "Glibc", "WinSDK"]),
    ("dart", &["dart:io"]),
    ("haskell", &["System.Posix", "System.Win32"]),
    ("ocaml", &["Unix"]),
    ("julia", &["Base.Libc"]),
    ("perl", &["POSIX"]),
    ("lua", &["os"]),
    ("ruby", &["Etc", "Fcntl", "Win32API"]),
    ("csharp", &["Microsoft.Win32"]),
    ("erlang", &["os"]),
];

/// C-family headers that are the operating system's interface (POSIX,
/// Linux, BSD, Windows), not the ISO C/C++ library. A header listed in
/// neither table, written in angle brackets, is a third-party library.
pub const SYSTEM_HEADERS: &[&str] = &[
    "unistd.h",
    "fcntl.h",
    "dlfcn.h",
    "pthread.h",
    "signal.h",
    "poll.h",
    "termios.h",
    "syslog.h",
    "pwd.h",
    "grp.h",
    "dirent.h",
    "glob.h",
    "fnmatch.h",
    "netdb.h",
    "ifaddrs.h",
    "spawn.h",
    "sched.h",
    "semaphore.h",
    "mqueue.h",
    "aio.h",
    "libgen.h",
    "utmp.h",
    "utmpx.h",
    "elf.h",
    "link.h",
    "err.h",
    "paths.h",
    "sysexits.h",
    "ucontext.h",
    "execinfo.h",
    "malloc.h",
    "alloca.h",
    "endian.h",
    "byteswap.h",
    "getopt.h",
    "pty.h",
    "wordexp.h",
    "ftw.h",
    "iconv.h",
    "langinfo.h",
    "nl_types.h",
    "regex.h",
    "strings.h",
    "ulimit.h",
    "utime.h",
    "windows.h",
    "winsock2.h",
    "ws2tcpip.h",
    "winbase.h",
    "winuser.h",
    "winnt.h",
    "windef.h",
    "winternl.h",
    "shellapi.h",
    "shlobj.h",
    "tlhelp32.h",
    "psapi.h",
    "io.h",
    "direct.h",
    "process.h",
    "conio.h",
    "mach/",
    "sys/",
    "linux/",
    "asm/",
    "netinet/",
    "arpa/",
    "net/",
    "bits/",
    "machine/",
    "libkern/",
    "CoreFoundation/",
    "IOKit/",
    "objbase.h",
    "ole2.h",
    "wincrypt.h",
    // Windows SDK API-set headers.
    "fileapi.h",
    "processenv.h",
    "wtypes.h",
    "shlwapi.h",
    "synchapi.h",
    "handleapi.h",
    "memoryapi.h",
    "libloaderapi.h",
    "sysinfoapi.h",
    "errhandlingapi.h",
    "processthreadsapi.h",
    "timezoneapi.h",
    "consoleapi.h",
    "wincon.h",
    "winerror.h",
    "winreg.h",
    "ntstatus.h",
    "sddl.h",
    "aclapi.h",
    "userenv.h",
    "lmcons.h",
    "iphlpapi.h",
    "winioctl.h",
    "winsvc.h",
    "wtsapi32.h",
    "dbghelp.h",
    "versionhelpers.h",
    "winver.h",
    "commctrl.h",
    "commdlg.h",
    "shellscalingapi.h",
    "dwmapi.h",
    "mmsystem.h",
    "winhttp.h",
    "wininet.h",
    "bcrypt.h",
    "ncrypt.h",
    "ntsecapi.h",
    "securitybaseapi.h",
    "minwindef.h",
    "basetsd.h",
    "intsafe.h",
    "tchar.h",
    "stringapiset.h",
    "heapapi.h",
    "ioapiset.h",
    "namedpipeapi.h",
    "realtimeapiset.h",
    "jobapi.h",
    "wow64apiset.h",
];

/// C-family OS headers of Windows; every other OS header is POSIX/Unix.
pub fn c_system_header_family(header: &str) -> &'static str {
    let h = header
        .trim()
        .trim_matches(['<', '>', '"'])
        .to_ascii_lowercase();
    let win = h.starts_with("win")
        || h.ends_with("api.h")
        || h.ends_with("apiset.h")
        || matches!(
            h.as_str(),
            "ws2tcpip.h"
                | "shellapi.h"
                | "shlobj.h"
                | "tlhelp32.h"
                | "psapi.h"
                | "io.h"
                | "direct.h"
                | "process.h"
                | "conio.h"
                | "objbase.h"
                | "ole2.h"
                | "wtypes.h"
                | "shlwapi.h"
                | "ntstatus.h"
                | "sddl.h"
                | "aclapi.h"
                | "userenv.h"
                | "lmcons.h"
                | "iphlpapi.h"
                | "dbghelp.h"
                | "versionhelpers.h"
                | "commctrl.h"
                | "commdlg.h"
                | "dwmapi.h"
                | "mmsystem.h"
                | "bcrypt.h"
                | "ncrypt.h"
                | "ntsecapi.h"
                | "minwindef.h"
                | "basetsd.h"
                | "intsafe.h"
                | "tchar.h"
                | "wtsapi32.h"
        );
    if win { "win32" } else { "posix" }
}

/// ISO C / C++ standard headers.
pub const ISO_C_HEADERS: &[&str] = &[
    "assert.h",
    "complex.h",
    "ctype.h",
    "errno.h",
    "fenv.h",
    "float.h",
    "inttypes.h",
    "iso646.h",
    "limits.h",
    "locale.h",
    "math.h",
    "setjmp.h",
    "stdalign.h",
    "stdarg.h",
    "stdatomic.h",
    "stdbool.h",
    "stddef.h",
    "stdint.h",
    "stdio.h",
    "stdlib.h",
    "stdnoreturn.h",
    "string.h",
    "tgmath.h",
    "threads.h",
    "time.h",
    "uchar.h",
    "wchar.h",
    "wctype.h",
    "stdbit.h",
    "stdckdint.h",
    "cassert",
    "cctype",
    "cerrno",
    "cfloat",
    "climits",
    "clocale",
    "cmath",
    "csetjmp",
    "csignal",
    "cstdarg",
    "cstddef",
    "cstdint",
    "cstdio",
    "cstdlib",
    "cstring",
    "ctime",
    "cwchar",
    "cwctype",
    "cinttypes",
    "algorithm",
    "any",
    "array",
    "atomic",
    "bit",
    "bitset",
    "charconv",
    "chrono",
    "compare",
    "complex",
    "concepts",
    "condition_variable",
    "coroutine",
    "deque",
    "exception",
    "execution",
    "expected",
    "filesystem",
    "format",
    "forward_list",
    "fstream",
    "functional",
    "future",
    "initializer_list",
    "iomanip",
    "ios",
    "iosfwd",
    "iostream",
    "istream",
    "iterator",
    "latch",
    "limits",
    "list",
    "locale",
    "map",
    "memory",
    "memory_resource",
    "mutex",
    "new",
    "numbers",
    "numeric",
    "optional",
    "ostream",
    "print",
    "queue",
    "random",
    "ranges",
    "ratio",
    "regex",
    "scoped_allocator",
    "semaphore",
    "set",
    "shared_mutex",
    "source_location",
    "span",
    "sstream",
    "stack",
    "stacktrace",
    "stdexcept",
    "stop_token",
    "streambuf",
    "string",
    "string_view",
    "syncstream",
    "system_error",
    "thread",
    "tuple",
    "type_traits",
    "typeindex",
    "typeinfo",
    "unordered_map",
    "unordered_set",
    "utility",
    "valarray",
    "variant",
    "vector",
    "version",
    "mdspan",
    "generator",
    "flat_map",
    "flat_set",
    "spanstream",
    "stdfloat",
];

/// POSIX, Linux and Win32 functions: the OS interface reached from
/// C-family code. A bare external call of one of these names is `system`.
pub const SYSTEM_APIS: &[&str] = &[
    "open",
    "openat",
    "close",
    "read",
    "write",
    "pread",
    "pwrite",
    "readv",
    "writev",
    "lseek",
    "dup",
    "dup2",
    "dup3",
    "pipe",
    "pipe2",
    "fcntl",
    "ioctl",
    "fsync",
    "fdatasync",
    "ftruncate",
    "truncate",
    "stat",
    "fstat",
    "lstat",
    "fstatat",
    "statx",
    "access",
    "faccessat",
    "chmod",
    "fchmod",
    "chown",
    "fchown",
    "lchown",
    "umask",
    "link",
    "symlink",
    "readlink",
    "unlink",
    "unlinkat",
    "rename",
    "renameat",
    "mkdir",
    "mkdirat",
    "rmdir",
    "chdir",
    "fchdir",
    "getcwd",
    "opendir",
    "readdir",
    "closedir",
    "mmap",
    "munmap",
    "mprotect",
    "msync",
    "madvise",
    "mlock",
    "munlock",
    "brk",
    "sbrk",
    "fork",
    "vfork",
    "clone",
    "execve",
    "_exit",
    "wait",
    "waitpid",
    "waitid",
    "kill",
    "raise",
    "sigaction",
    "sigprocmask",
    "sigsuspend",
    "sigaltstack",
    "signal",
    "alarm",
    "pause",
    "sleep",
    "usleep",
    "nanosleep",
    "getpid",
    "getppid",
    "getuid",
    "geteuid",
    "getgid",
    "getegid",
    "setuid",
    "setgid",
    "setsid",
    "getpgid",
    "setpgid",
    "getrlimit",
    "setrlimit",
    "getrusage",
    "prctl",
    "ptrace",
    "socket",
    "socketpair",
    "bind",
    "listen",
    "accept",
    "accept4",
    "connect",
    "send",
    "sendto",
    "sendmsg",
    "recv",
    "recvfrom",
    "recvmsg",
    "shutdown",
    "setsockopt",
    "getsockopt",
    "getsockname",
    "getpeername",
    "getaddrinfo",
    "freeaddrinfo",
    "gethostbyname",
    "select",
    "pselect",
    "poll",
    "ppoll",
    "epoll_create",
    "epoll_create1",
    "epoll_ctl",
    "epoll_wait",
    "kqueue",
    "kevent",
    "inotify_init",
    "inotify_init1",
    "inotify_add_watch",
    "eventfd",
    "timerfd_create",
    "signalfd",
    "syscall",
    "sysconf",
    "pathconf",
    "uname",
    "gethostname",
    "clock_gettime",
    "clock_nanosleep",
    "gettimeofday",
    "pthread_create",
    "pthread_join",
    "pthread_detach",
    "pthread_mutex_lock",
    "pthread_mutex_unlock",
    "pthread_cond_wait",
    "pthread_cond_signal",
    "pthread_kill",
    "sched_yield",
    "sched_setaffinity",
    "sem_open",
    "sem_wait",
    "sem_post",
    "shm_open",
    "shm_unlink",
    "mq_open",
    "dlsym",
    "dlclose",
    "dlerror",
    "dladdr",
    "isatty",
    "ttyname",
    "tcgetattr",
    "tcsetattr",
    "openpty",
    "chroot",
    "mount",
    "umount",
    "sync",
    "reboot",
    "getenv",
    "setenv",
    "unsetenv",
    "putenv",
    "CreateFileA",
    "CreateFileW",
    "ReadFile",
    "WriteFile",
    "CloseHandle",
    "DeviceIoControl",
    "VirtualAlloc",
    "VirtualFree",
    "VirtualProtect",
    "HeapAlloc",
    "HeapFree",
    "GetLastError",
    "SetLastError",
    "CreateThread",
    "ExitThread",
    "WaitForSingleObject",
    "WaitForMultipleObjects",
    "CreateEventA",
    "CreateEventW",
    "SetEvent",
    "ResetEvent",
    "CreateMutexA",
    "CreateMutexW",
    "ReleaseMutex",
    "Sleep",
    "GetTickCount",
    "GetTickCount64",
    "QueryPerformanceCounter",
    "GetProcAddress",
    "FreeLibrary",
    "GetModuleFileNameA",
    "GetModuleFileNameW",
    "RegOpenKeyExA",
    "RegOpenKeyExW",
    "RegQueryValueExA",
    "RegQueryValueExW",
    "RegSetValueExA",
    "RegSetValueExW",
    "RegCloseKey",
    "MessageBoxA",
    "MessageBoxW",
    "GetCurrentProcess",
    "GetCurrentProcessId",
    "TerminateProcess",
    "OpenProcess",
    "GetEnvironmentVariableA",
    "GetEnvironmentVariableW",
    "WSAStartup",
    "WSACleanup",
    "NtQuerySystemInformation",
    "ZwQuerySystemInformation",
];

/// Assembly instructions that enter the kernel.
pub const SYSCALL_MNEMONICS: &[&str] = &[
    "syscall", "sysenter", "svc", "swi", "ecall", "sc", "trap", "int",
];

/// Shell builtins and keywords: a command of one of these names runs in
/// the shell itself. Any other command a shell script runs is a program.
pub const SHELL_BUILTINS: &[&str] = &[
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "help",
    "history",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "wait",
    ".",
    ":",
    "[",
    "[[",
    "if",
    "then",
    "else",
    "elif",
    "fi",
    "for",
    "while",
    "until",
    "do",
    "done",
    "case",
    "esac",
    "function",
    "select",
    "time",
    "coproc",
];

/// The per-language entries of one of the tables above.
pub fn externals_table(
    table: &'static [(&'static str, &'static [&'static str])],
    lang: &str,
) -> &'static [&'static str] {
    let lang = if lang == "kivy" { "python" } else { lang };
    table
        .iter()
        .find(|(l, _)| *l == lang)
        .map_or(&[], |(_, v)| v)
}

/// The library a stdlib call with no import evidence belongs to.
pub fn stdlib_library_name(lang: &str) -> &'static str {
    STDLIB_LIBRARY_NAMES
        .iter()
        .find(|(l, _)| *l == lang)
        .map_or("stdlib", |(_, n)| n)
}

/// Whether `module` is under one of the language's [`STDLIB_ROOTS`].
pub fn under_stdlib_root(lang: &str, module: &str) -> bool {
    let module = module.trim().trim_start_matches("::");
    externals_table(STDLIB_ROOTS, lang).iter().any(|root| {
        if root.ends_with([':', '/']) {
            return module.starts_with(root);
        }
        module == *root
            || module
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with(['.', ':', '/', '\\']))
    })
}

/// Whether a C-family header is ISO C/C++ (`Some(false)`), the OS
/// interface (`Some(true)`), or neither (`None`).
pub fn c_header_class(header: &str) -> Option<bool> {
    let h = header.trim().trim_matches(['<', '>', '"']);
    if ISO_C_HEADERS.contains(&h) {
        return Some(false);
    }
    if SYSTEM_HEADERS.iter().any(|s| {
        if s.ends_with('/') {
            h.starts_with(s)
        } else {
            h == *s
        }
    }) {
        return Some(true);
    }
    None
}
