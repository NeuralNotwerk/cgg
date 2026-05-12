//! Stdlib manifests — embedded at compile time.
//!
//! Each manifest is a newline-separated list of known stdlib symbol names
//! for a given language. Used by the external call classifier to identify
//! calls that target stdlib/framework code even when the callee name
//! happens to match a project-defined symbol.

use std::collections::HashSet;
use std::sync::OnceLock;

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
        ]
    });
    all.iter().find(|(id, _)| *id == lang).map(|(_, set)| set)
}
