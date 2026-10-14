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
/// class they are called on: Ruby's `Foo.new(…)` builds a `Foo`. A
/// receiver written as one is an instance of that class.
pub const CONSTRUCTING_METHODS: &[(&str, &[&str])] = &[("ruby", &["new"])];

/// Whether `name` is one of `lang`'s [`CONSTRUCTING_METHODS`].
pub fn is_constructing_method(lang: &str, name: &str) -> bool {
    CONSTRUCTING_METHODS
        .iter()
        .any(|(l, names)| *l == lang && names.contains(&name))
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

/// Types every value has: a parameter declared as one says nothing about
/// which class a method called on it belongs to.
pub const TOP_TYPES: &[(&str, &[&str])] = &[
    ("scala", &["Any", "AnyRef", "AnyVal"]),
    ("csharp", &["Object", "Dynamic"]),
    ("dart", &["Object", "Dynamic"]),
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
        ]
    });
    all.iter().find(|(id, _)| *id == lang).map(|(_, set)| set)
}
