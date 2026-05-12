pub mod rust;
pub mod python;
pub mod javascript;
pub mod typescript;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod c;
pub mod cpp;
pub mod csharp;
pub mod bash;
pub mod ruby;
pub mod swift;
pub mod lua;
pub mod php;
pub mod dart;
pub mod scala;
pub mod hcl;
pub mod zig;
pub mod objc;
pub mod r;
pub mod groovy;
pub mod julia;
pub mod perl;
pub mod elixir;
pub mod erlang;
pub mod fortran;
pub mod clojure;
pub mod haskell;
pub mod ocaml;

use crate::PluginRegistry;

/// Extract the signature from a function/method node's full text.
///
/// Takes everything up to (but not including) the opening body delimiter
/// (`{`, or `:` followed by newline for Python), collapses internal
/// whitespace, and trims trailing artifacts.
pub fn extract_signature(full_text: &str) -> String {
    // Find the body start: first `{` at depth 0, or `:` at depth 0
    // that is followed by a newline (Python/Ruby body delimiter).
    let mut depth = 0i32;
    let mut sig_end = full_text.len();
    for (i, ch) in full_text.char_indices() {
        match ch {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth = (depth - 1).max(0),
            '{' if depth == 0 => { sig_end = i; break; }
            ':' if depth == 0 => {
                // Only treat as body delimiter if followed by newline/space+code
                // (not a type annotation like `x: int`)
                let rest = &full_text[i + 1..];
                let next_non_ws = rest.trim_start();
                if rest.starts_with('\n') || rest.starts_with("\r\n")
                    || (next_non_ws.len() < rest.len() && !next_non_ws.is_empty()
                        && !next_non_ws.starts_with(':'))
                {
                    // Check if we're past the closing paren (signature is complete)
                    let before = &full_text[..i];
                    let open = before.chars().filter(|&c| c == '(').count();
                    let close = before.chars().filter(|&c| c == ')').count();
                    if open > 0 && open == close {
                        sig_end = i;
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let raw = &full_text[..sig_end];
    // Collapse whitespace (newlines, indentation) into single spaces.
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches('{').trim_end_matches(':').trim_end_matches('=').trim().to_string()
}

/// Register every v1 plugin into `reg`.
pub fn register_all(reg: &mut PluginRegistry) {
    reg.register(Box::new(rust::RustPlugin));
    reg.register(Box::new(python::PythonPlugin));
    reg.register(Box::new(javascript::JavaScriptPlugin));
    reg.register(Box::new(typescript::TypeScriptPlugin));
    reg.register(Box::new(go::GoPlugin));
    reg.register(Box::new(java::JavaPlugin));
    reg.register(Box::new(kotlin::KotlinPlugin));
    reg.register(Box::new(c::CPlugin));
    reg.register(Box::new(cpp::CppPlugin));
    reg.register(Box::new(csharp::CSharpPlugin));
    reg.register(Box::new(bash::BashPlugin));
    reg.register(Box::new(ruby::RubyPlugin));
    reg.register(Box::new(swift::SwiftPlugin));
    reg.register(Box::new(lua::LuaPlugin));
    reg.register(Box::new(php::PhpPlugin));
    reg.register(Box::new(dart::DartPlugin));
    reg.register(Box::new(scala::ScalaPlugin));
    reg.register(Box::new(hcl::HclPlugin));
    reg.register(Box::new(zig::ZigPlugin));
    reg.register(Box::new(objc::ObjcPlugin));
    reg.register(Box::new(r::RPlugin));
    reg.register(Box::new(groovy::GroovyPlugin));
    reg.register(Box::new(julia::JuliaPlugin));
    reg.register(Box::new(perl::PerlPlugin));
    reg.register(Box::new(elixir::ElixirPlugin));
    reg.register(Box::new(erlang::ErlangPlugin));
    reg.register(Box::new(fortran::FortranPlugin));
    reg.register(Box::new(clojure::ClojurePlugin));
    reg.register(Box::new(haskell::HaskellPlugin));
    reg.register(Box::new(ocaml::OcamlPlugin));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_plugin_returns_a_language() {
        let mut reg = PluginRegistry::new();
        register_all(&mut reg);
        for p in reg.all() {
            let _ = p.ts_language();
            assert!(!p.extensions().is_empty(), "{} has no extensions", p.id());
        }
    }
}
