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
    reg.register(Box::new(ruby::RubyPlugin));
    reg.register(Box::new(swift::SwiftPlugin));
    reg.register(Box::new(lua::LuaPlugin));
    reg.register(Box::new(php::PhpPlugin));
    reg.register(Box::new(dart::DartPlugin));
    reg.register(Box::new(scala::ScalaPlugin));
    reg.register(Box::new(hcl::HclPlugin));
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
