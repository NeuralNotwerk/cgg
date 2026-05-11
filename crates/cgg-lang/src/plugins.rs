//! Per-language plugin registrations.
//!
//! Each plugin lives in a sibling file; this module wires them into
//! the [`PluginRegistry`]. Full extraction logic for Rust and Python
//! lands in their respective `extract.rs` files (Task 4). Other
//! languages currently return empty [`FileFacts`]; they pick up real
//! extraction in Task 7a (JS/TS), Task 7 (C/C++), and Task 6b (Go, C#).

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

use crate::PluginRegistry;

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
