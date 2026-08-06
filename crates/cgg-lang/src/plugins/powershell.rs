//! PowerShell plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct PowerShellPlugin;

impl LanguagePlugin for PowerShellPlugin {
    fn id(&self) -> &'static str { "powershell" }
    fn extensions(&self) -> &'static [&'static str] { &[".ps1", ".psm1", ".psd1"] }
    fn shebangs(&self) -> &'static [&'static str] { &["pwsh", "powershell"] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_powershell::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "powershell");
        let mut w = PowerShellWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct PowerShellWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> PowerShellWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}.{simple}", self.scope.join(".")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_statement" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "class_statement" => {
                let name = self.child_kind(node, "simple_name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else { self.walk_children(node); }
                return;
            }
            "class_method_definition" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            "command" => {
                self.record_command(node);
                self.walk_children(node);
                return;
            }
            "invokation_expression" => {
                self.record_invokation(node);
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn child_kind<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                if c.node().kind() == kind { return Some(c.node()); }
                if !c.goto_next_sibling() { break; }
            }
        }
        None
    }

    fn record_function(&mut self, node: Node) {
        let Some(name_node) = self.child_kind(node, "function_name") else { return };
        let name = self.text(name_node).to_string();
        if name.is_empty() { return; }
        let qn = self.qn(&name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: if self.scope.is_empty() { DefVariant::FreeFunction } else { DefVariant::InherentMethod },
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(), attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_method(&mut self, node: Node) {
        // class_method_definition layout: [class_attribute ("static")?] [type_literal]? simple_name (...) { ... }
        // The method name is the FIRST simple_name child that isn't inside a type_literal.
        let mut name_node: Option<Node> = None;
        let mut is_static = false;
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let n = c.node();
                match n.kind() {
                    "class_attribute" => {
                        if self.text(n).contains("static") { is_static = true; }
                    }
                    "simple_name" if name_node.is_none() => {
                        name_node = Some(n);
                    }
                    _ => {}
                }
                if !c.goto_next_sibling() { break; }
            }
        }
        let Some(name_node) = name_node else { return };
        let name = self.text(name_node).to_string();
        if name.is_empty() { return; }
        // Constructor: name equals enclosing class name.
        let is_ctor = self.scope.last().map(|s| s == &name).unwrap_or(false);
        let variant = if is_ctor { DefVariant::Constructor }
            else if is_static { DefVariant::StaticMethod }
            else { DefVariant::InherentMethod };
        let qn = self.qn(&name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name, qualified_name: qn, variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(), attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_command(&mut self, node: Node) {
        // `command_name` may sit directly under `command` or one level down inside
        // `command_name_expr` (dot-source form: `. .\path.ps1`).
        let name_node = self.child_kind(node, "command_name")
            .or_else(|| self.child_kind(node, "command_name_expr")
                .and_then(|wrap| self.child_kind(wrap, "command_name")));
        let Some(name_node) = name_node else { return };
        let name = self.text(name_node).trim().to_string();
        if name.is_empty() { return; }

        // `using namespace|module|assembly X` is parsed as a command whose name is "using".
        if name.eq_ignore_ascii_case("using") {
            self.record_using(node);
            return;
        }
        // `Import-Module X` is a command.
        if name.eq_ignore_ascii_case("import-module") {
            if let Some(path) = self.first_command_argument(node) {
                self.facts.imports.push(ImportRecord {
                    kind: "import-module".into(),
                    path,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
            return;
        }
        // Dot-sourcing: `. .\file.ps1` — command_invokation_operator is `.`, name is the path.
        if let Some(op) = self.child_kind(node, "command_invokation_operator") {
            if self.text(op).trim() == "." {
                self.facts.imports.push(ImportRecord {
                    kind: "dot-source".into(),
                    path: name,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
                return;
            }
        }

        self.facts.references.push(RefRecord {
            name, receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }

    fn record_using(&mut self, node: Node) {
        // `using <namespace|module|assembly> <target>` — flat command_elements.
        let Some(elements) = self.child_kind(node, "command_elements") else { return };
        let mut kind: Option<String> = None;
        let mut target: Option<String> = None;
        let mut c = elements.walk();
        if c.goto_first_child() {
            loop {
                let n = c.node();
                if n.kind() == "generic_token" {
                    let text = self.text(n).trim().to_string();
                    if kind.is_none() {
                        kind = Some(text);
                    } else if target.is_none() {
                        target = Some(text);
                    }
                }
                if !c.goto_next_sibling() { break; }
            }
        }
        if let (Some(kind), Some(target)) = (kind, target) {
            self.facts.imports.push(ImportRecord {
                kind: format!("using-{}", kind.to_ascii_lowercase()),
                path: target,
                alias: String::new(),
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn first_command_argument(&self, node: Node) -> Option<String> {
        let elements = self.child_kind(node, "command_elements")?;
        let mut c = elements.walk();
        if c.goto_first_child() {
            loop {
                let n = c.node();
                if matches!(n.kind(), "generic_token" | "command_name") {
                    let text = self.text(n).trim().trim_matches('"').trim_matches('\'').to_string();
                    if !text.is_empty() { return Some(text); }
                }
                if !c.goto_next_sibling() { break; }
            }
        }
        None
    }

    fn record_invokation(&mut self, node: Node) {
        // invokation_expression: receiver (variable|type_literal) [.|::] member_name argument_list
        let member = self.child_kind(node, "member_name")
            .and_then(|m| self.child_kind(m, "simple_name").or(Some(m)))
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if member.is_empty() { return; }

        let mut receiver = String::new();
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let n = c.node();
                match n.kind() {
                    "variable" => { receiver = self.text(n).trim_start_matches('$').to_string(); break; }
                    "type_literal" => {
                        // e.g. [Service] — capture the inner type name
                        if let Some(spec) = self.child_kind(n, "type_spec") {
                            receiver = self.text(spec).to_string();
                        } else {
                            receiver = self.text(n).trim_matches(|c: char| c == '[' || c == ']').to_string();
                        }
                        break;
                    }
                    "member_name" => break,
                    _ => {}
                }
                if !c.goto_next_sibling() { break; }
            }
        }

        self.facts.references.push(RefRecord {
            name: member, receiver_hint: receiver,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_powershell::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        PowerShellPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.ps1"), &tree, src.as_bytes())
    }

    #[test]
    fn free_function() {
        let src = "function Get-Greeting { param([string]$Name) Write-Host \"hi $Name\" }\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "Get-Greeting" && d.variant == DefVariant::FreeFunction),
            "defs: {:?}", f.definitions);
    }

    #[test]
    fn filter_is_callable() {
        let src = "filter Double { $_ * 2 }\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "Double"));
    }

    #[test]
    fn class_methods() {
        let src = "\
class Service {
    [string]$Name
    Service([string]$n) { $this.Name = $n }
    [void] Run() { Write-Host \"x\" }
    static [Service] Create() { return [Service]::new(\"x\") }
}
";
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f.definitions.iter()
            .map(|d| (d.qualified_name.clone(), d.variant)).collect();
        assert_eq!(by.get("Service.Service"), Some(&DefVariant::Constructor), "by: {by:?}");
        assert_eq!(by.get("Service.Run"), Some(&DefVariant::InherentMethod));
        assert_eq!(by.get("Service.Create"), Some(&DefVariant::StaticMethod));
    }

    #[test]
    fn cmdlet_call_captured() {
        let src = "Get-Greeting -Name \"world\"\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "Get-Greeting"),
            "refs: {:?}", f.references);
    }

    #[test]
    fn method_invokation_captured() {
        let src = "$svc.Run()\n[Service]::Create()\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "Run" && r.receiver_hint == "svc"),
            "refs: {:?}", f.references);
        assert!(f.references.iter().any(|r| r.name == "Create" && r.receiver_hint == "Service"),
            "refs: {:?}", f.references);
    }

    #[test]
    fn import_module_captured() {
        let src = "Import-Module Foo\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path == "Foo" && i.kind == "import-module"),
            "imports: {:?}", f.imports);
    }

    #[test]
    fn using_namespace_captured() {
        let src = "using namespace System.IO\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path == "System.IO" && i.kind == "using-namespace"),
            "imports: {:?}", f.imports);
    }

    #[test]
    fn dot_source_captured() {
        let src = ". .\\helper.ps1\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.kind == "dot-source"),
            "imports: {:?}", f.imports);
    }
}
