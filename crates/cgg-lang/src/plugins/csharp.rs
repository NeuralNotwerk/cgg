//! C# plugin.
//!
//! Two-phase AST pass over `tree-sitter-c-sharp`:
//!
//! * **Definitions** — `class_declaration`, `struct_declaration`,
//!   `record_declaration`, `interface_declaration` contribute to the
//!   scope stack (they don't themselves produce callables, but their
//!   methods do). `method_declaration`, `constructor_declaration`,
//!   `destructor_declaration`, and `local_function_statement` become
//!   callables. Accessors (`get`/`set`) of a property are emitted as
//!   `Property` variants.
//! * **References** — `invocation_expression` with callee either an
//!   `identifier` or a `member_access_expression` (`obj.Foo`,
//!   `Ns.Sub.Foo`).
//! * **Imports** — `using_directive` records: plain `using Ns;`,
//!   aliased `using A = Ns.X;`, and `using static Ns.X;` are all
//!   captured; v1 resolves only the plain and aliased forms.
//!
//! Limitations: partial classes are merged only by exact name
//! (behavior matches most IDEs at the scope level, but cross-file
//! overload resolution is not modeled); generics, extension methods
//! with special dispatch, and explicit interface implementations are
//! best-effort.

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &'static str {
        "csharp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cs", ".csx"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            visibility: true,
            attributes: true,
            impls: true,
            value_refs: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c_sharp::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "csharp");
        let mut w = Walker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

#[derive(Clone, Debug)]
enum Scope {
    Namespace(String),
    Type(String),
}

impl Scope {
    fn name(&self) -> &str {
        match self {
            Scope::Namespace(s) | Scope::Type(s) => s.as_str(),
        }
    }
}

struct Walker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    scope: Vec<Scope>,
    /// Base list of the enclosing type, innermost last — `: IJob` is
    /// what makes `Execute` an entry point, and only the type says so.
    bases: Vec<Vec<String>>,
}

impl<'a> Walker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    // The full namespace, nested blocks joined, so the
                    // resolver knows which namespaces the workspace
                    // declares — including a file-scoped one, whose
                    // members' qualified names do not carry it.
                    let mut full: Vec<&str> = self
                        .scope
                        .iter()
                        .filter_map(|s| match s {
                            Scope::Namespace(n) => Some(n.as_str()),
                            Scope::Type(_) => None,
                        })
                        .collect();
                    full.push(name.as_str());
                    let full = full.join(".");
                    if !self.facts.packages.contains(&full) {
                        self.facts.packages.push(full);
                    }
                    self.scope.push(Scope::Namespace(name));
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "class_declaration"
            | "struct_declaration"
            | "record_declaration"
            | "record_struct_declaration"
            | "interface_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let bases = super::attrs::base_types(node, self.source);
                self.bases.push(bases);
                if !name.is_empty() {
                    self.scope.push(Scope::Type(name));
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                self.bases.pop();
                return;
            }
            "method_declaration" | "local_function_statement" => {
                self.record_method(node, DefVariant::InherentMethod);
                self.walk_children(node);
                return;
            }
            "constructor_declaration" => {
                self.record_method(node, DefVariant::Constructor);
                self.walk_children(node);
                return;
            }
            "destructor_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| format!("~{}", self.text(n)))
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_named(node, &name, DefVariant::Destructor);
                }
                self.walk_children(node);
                return;
            }
            "accessor_declaration" => {
                // get/set accessors inside a property.
                let name = node
                    .children(&mut node.walk())
                    .find(|c| matches!(c.kind(), "get" | "set" | "init"))
                    .map(|n| n.kind().to_string())
                    .unwrap_or_else(|| "accessor".to_string());
                self.record_named(node, &name, DefVariant::Property);
                self.walk_children(node);
                return;
            }
            "using_directive" => {
                self.record_using(node);
                return;
            }
            "local_declaration_statement" => {
                self.record_local_type(node);
                self.walk_children(node);
                return;
            }
            "invocation_expression" => {
                if let Some(r) = self.ref_from_invoke(node) {
                    // Shape B: `app.MapGet("/x", Handler)` — the minimal
                    // API's whole routing surface lives in argument
                    // position, so the callee alone tells us nothing.
                    let context = if r.receiver_hint.is_empty() {
                        r.name.clone()
                    } else {
                        format!("{}.{}", r.receiver_hint, r.name)
                    };
                    self.facts.references.push(r);
                    let extra =
                        super::registrar::capture(&self.ctx, node, self.source, &context);
                    self.facts.references.extend(extra);
                }
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                self.walk(cursor.node());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_method(&mut self, node: Node, variant: DefVariant) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }
        self.record_named(node, &simple, variant);
    }

    fn record_named(&mut self, node: Node, simple: &str, variant: DefVariant) {
        let mut parts: Vec<&str> = self.scope.iter().map(|s| s.name()).collect();
        parts.push(simple);
        let qn = parts.join(".");
        let (sl, el) = line_range(node);
        self.facts.definitions.push(DefRecord {
            simple_name: simple.to_string(),
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis: csharp_vis(node),
            // Verbatim, so `[HttpGet("/users")]` keeps its route.
            attributes: super::attrs::collect(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_local_type(&mut self, node: Node) {
        // local_declaration_statement -> variable_declaration -> type + variable_declarator
        let var_decl = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "variable_declaration");
        let Some(var_decl) = var_decl else { return };

        // Check if explicit type or implicit (var)
        let mut children = var_decl.walk();
        let type_node = var_decl.children(&mut children).find(|c| {
            c.kind() == "identifier"
                || c.kind() == "qualified_name"
                || c.kind() == "generic_name"
                || c.kind() == "implicit_type"
        });
        let Some(type_node) = type_node else { return };

        let declarator = var_decl
            .children(&mut var_decl.walk())
            .find(|c| c.kind() == "variable_declarator");
        let Some(declarator) = declarator else { return };
        let var_name = declarator
            .children(&mut declarator.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if var_name.is_empty() {
            return;
        }

        if type_node.kind() == "implicit_type" {
            // `var x = new Foo(...)` — infer type from object_creation_expression
            let new_expr = declarator
                .children(&mut declarator.walk())
                .find(|c| c.kind() == "object_creation_expression");
            if let Some(new_expr) = new_expr {
                let type_id = new_expr
                    .child_by_field_name("type")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !type_id.is_empty() && type_id.starts_with(char::is_uppercase) {
                    self.facts.local_types.push(cgg_core::LocalType {
                        var_name,
                        type_name: type_id,
                        scope_byte: node.start_byte() as u32,
                    });
                }
            }
        } else {
            // Explicit type: `Service svc = ...`
            let type_name = self.text(type_node).to_string();
            if !type_name.is_empty() && type_name.starts_with(char::is_uppercase) {
                self.facts.local_types.push(cgg_core::LocalType {
                    var_name,
                    type_name,
                    scope_byte: node.start_byte() as u32,
                });
            }
        }
    }

    fn record_using(&mut self, node: Node) {
        // `using A;` | `using A = B;` | `using static A;`
        let text = self.text(node).trim().to_string();
        let is_static = text.starts_with("using static ");
        let body = text
            .trim_start_matches("using")
            .trim()
            .trim_start_matches("static")
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();

        let (path, alias) = if let Some((lhs, rhs)) = body.split_once('=') {
            (rhs.trim().to_string(), lhs.trim().to_string())
        } else {
            (body.clone(), String::new())
        };

        let kind = if is_static { "using-static" } else { "using" };
        self.facts.imports.push(ImportRecord {
            kind: kind.into(),
            path,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn ref_from_invoke(&mut self, node: Node) -> Option<RefRecord> {
        let func = node.child_by_field_name("function")?;
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "member_access_expression" => {
                let expr = func.child_by_field_name("expression")?;
                let name = func.child_by_field_name("name")?;
                (self.text(name).to_string(), self.text(expr).to_string())
            }
            "generic_name" => {
                // Foo<T>() — take the base identifier.
                let base = func
                    .children(&mut func.walk())
                    .find(|c| c.kind() == "identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (base, String::new())
            }
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        let site_line = (node.start_position().row as u32) + 1;
        Some(RefRecord {
            name,
            receiver_hint: recv,
            site_line,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        })
    }
}

fn line_range(n: Node) -> (u32, u32) {
    let s = (n.start_position().row as u32) + 1;
    let e = (n.end_position().row as u32) + 1;
    (s, e)
}

/// Does this declaration carry the modifier keyword `kw`?
///
/// Each `modifier` child wraps one keyword token; matching the token
/// rather than the text means an attribute argument
/// (`[Obsolete("x")] public void F()`) can neither fake a modifier nor
/// hide one.
fn has_modifier(node: Node, kw: &str) -> bool {
    node.children(&mut node.walk())
        .filter(|c| c.kind() == "modifier")
        .any(|m| m.children(&mut m.walk()).any(|t| t.kind() == kw))
}

/// Project the declaration's access modifiers onto the shared
/// vocabulary. The *absent* case is the interesting one and differs per
/// language — and, in C#, per enclosing declaration — which is exactly
/// why this normalization belongs in the plugin.
///
/// Read from the `modifier` nodes. It used to be read from the signature
/// text cut at the first `(`, which an attribute argument ends early:
/// `[Obsolete("x")] public void F()` read as private, and so as
/// corroborated dead code.
fn csharp_vis(node: Node) -> cgg_core::Vis {
    // `protected internal` and `private protected` both reach subtypes,
    // so `protected` wins over the keyword it is combined with.
    if has_modifier(node, "public") {
        return cgg_core::Vis::Public;
    }
    if has_modifier(node, "protected") {
        return cgg_core::Vis::Protected;
    }
    if has_modifier(node, "private") {
        return cgg_core::Vis::Private;
    }
    if has_modifier(node, "internal") {
        return cgg_core::Vis::Internal;
    }
    let parent = node.parent();
    match parent.map(|p| p.kind()) {
        // An accessor without its own modifier has the accessibility of
        // the property, indexer or event that declares it
        // (`accessor_list` → declaration).
        Some("accessor_list") => parent
            .and_then(|l| l.parent())
            .map(csharp_vis)
            .unwrap_or(cgg_core::Vis::Private),
        // Interface members are public unless they say otherwise (C# 8
        // lets a default-implemented member be `private`/`protected`,
        // handled above). Only the interface's own body counts; a type
        // nested inside it has a body of its own.
        Some("declaration_list")
            if parent
                .and_then(|l| l.parent())
                .is_some_and(|d| d.kind() == "interface_declaration") =>
        {
            cgg_core::Vis::Public
        }
        // Class, struct and record members default to private, and so
        // do declarations that cannot carry an access modifier at all:
        // local functions, static constructors, finalizers, and explicit
        // interface implementations (`void IFoo.Bar()`), which are
        // reachable only through the interface.
        _ => cgg_core::Vis::Private,
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
        p.set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        CSharpPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.cs"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn method_qualified_name_with_namespace_and_class() {
        let src = r#"
namespace App.Core {
    class Service {
        public void Run() {}
        void Helper() {}
    }
}
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"App.Core.Service.Run"), "got: {names:?}");
        assert!(names.contains(&"App.Core.Service.Helper"), "got: {names:?}");
    }

    #[test]
    fn constructor_and_destructor_variants() {
        let src = r#"
class Foo {
    public Foo() {}
    ~Foo() {}
}
"#;
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["Foo"], DefVariant::Constructor);
        assert_eq!(by["~Foo"], DefVariant::Destructor);
    }

    #[test]
    fn using_directive_captured() {
        let src =
            "using System; using Sys = System; using static System.Math;\nclass C {}\n";
        let f = extract(src);
        let usings: Vec<(String, String, String)> = f
            .imports
            .iter()
            .map(|i| (i.kind.clone(), i.path.clone(), i.alias.clone()))
            .collect();
        assert!(usings.contains(&("using".into(), "System".into(), "".into())));
        assert!(usings.contains(&("using".into(), "System".into(), "Sys".into())));
        assert!(usings.contains(&(
            "using-static".into(),
            "System.Math".into(),
            "".into()
        )));
    }

    #[test]
    fn invocation_references_extracted() {
        let src = r#"
using System;
class Service {
    public void Run() {
        Helper();
        Console.WriteLine("hi");
    }
    void Helper() {}
}
"#;
        let f = extract(src);
        let refs: Vec<&RefRecord> = f.references.iter().collect();
        assert!(
            refs.iter()
                .any(|r| r.name == "Helper" && r.receiver_hint.is_empty()),
            "no bare Helper call: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "WriteLine" && r.receiver_hint == "Console"),
            "no Console.WriteLine ref: {refs:?}"
        );
    }

    #[test]
    fn nested_namespaces_join_dots() {
        let src = r#"
namespace A.B.C {
    class T { void M() {} }
}
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        // `A.B.C` is a qualified_name node; our extractor uses the
        // text of the name field which includes the dots already.
        assert!(names.iter().any(|n| n.ends_with(".T.M")), "got: {names:?}");
        assert!(names.iter().any(|n| n.contains("A.B.C")), "got: {names:?}");
    }

    #[test]
    fn declared_namespaces_are_recorded_in_full() {
        let f = extract(
            "namespace A.B { namespace C { class T { void M() {} } } }\n\
             namespace D { }\n",
        );
        assert_eq!(f.packages, vec!["A.B", "A.B.C", "D"]);
        let scoped =
            extract("namespace Acme.Web;\npublic class App { public void Run() {} }\n");
        assert_eq!(scoped.packages, vec!["Acme.Web"]);
    }

    fn vis_of(f: &FileFacts, qn_suffix: &str) -> cgg_core::Vis {
        f.definitions
            .iter()
            .find(|d| d.qualified_name.ends_with(qn_suffix))
            .unwrap_or_else(|| panic!("{qn_suffix} not extracted"))
            .vis
    }

    #[test]
    fn an_attribute_argument_does_not_hide_the_access_modifier() {
        // FINDINGS B1: the signature was cut at the first `(`, which here
        // is the attribute's, so every modifier after it was lost and the
        // method read as private — corroborated dead code.
        use cgg_core::Vis;
        let f = extract(
            "class X {\n\
             \x20 [Obsolete(\"x\")]\n  public void Pub() {}\n\
             \x20 [Obsolete(\"x\", false)] protected void Prot() {}\n\
             \x20 [Obsolete(\"x\")] internal void Intl() {}\n\
             \x20 [Obsolete(\"x\")] private void Priv() {}\n\
             \x20 [Obsolete(\"x\")] void Dflt() {}\n\
             }\n",
        );
        assert_eq!(vis_of(&f, ".Pub"), Vis::Public);
        assert_eq!(vis_of(&f, ".Prot"), Vis::Protected);
        assert_eq!(vis_of(&f, ".Intl"), Vis::Internal);
        // Near-misses: an explicit `private`, or no modifier at all,
        // behind the same attribute must not change.
        assert_eq!(vis_of(&f, ".Priv"), Vis::Private);
        assert_eq!(vis_of(&f, ".Dflt"), Vis::Private);
    }

    #[test]
    fn modifier_words_outside_modifier_nodes_are_not_modifiers() {
        use cgg_core::Vis;
        let f = extract(
            "class X {\n  void A(string publicId) {}\n  [Named(\"public\")] void B() {}\n}\n",
        );
        assert_eq!(vis_of(&f, ".A"), Vis::Private);
        assert_eq!(vis_of(&f, ".B"), Vis::Private);
    }

    #[test]
    fn combined_access_modifiers_reach_subtypes() {
        use cgg_core::Vis;
        let f = extract(
            "class X {\n  protected internal void A() {}\n  private protected void B() {}\n}\n",
        );
        assert_eq!(vis_of(&f, ".A"), Vis::Protected);
        assert_eq!(vis_of(&f, ".B"), Vis::Protected);
    }

    #[test]
    fn interface_members_without_a_modifier_are_public() {
        use cgg_core::Vis;
        let f = extract(
            "public interface IRepo {\n\
             \x20 void FindAll();\n\
             \x20 [Obsolete(\"x\")] void Annotated();\n\
             \x20 void WithBody() {}\n\
             \x20 private void Helper() {}\n\
             \x20 protected void Hook() {}\n\
             \x20 int Count { get; }\n\
             \x20 class Nested { void Inner() {} }\n\
             }\n",
        );
        assert_eq!(vis_of(&f, "IRepo.FindAll"), Vis::Public);
        assert_eq!(vis_of(&f, "IRepo.Annotated"), Vis::Public);
        assert_eq!(vis_of(&f, "IRepo.WithBody"), Vis::Public);
        assert_eq!(vis_of(&f, "IRepo.get"), Vis::Public);
        // An explicit modifier still wins in an interface.
        assert_eq!(vis_of(&f, "IRepo.Helper"), Vis::Private);
        assert_eq!(vis_of(&f, "IRepo.Hook"), Vis::Protected);
        // Near-miss: a type nested in the interface has class rules.
        assert_eq!(vis_of(&f, "Nested.Inner"), Vis::Private);
    }

    #[test]
    fn class_struct_and_record_members_default_to_private() {
        use cgg_core::Vis;
        let f = extract(
            "class C { void M() {} }\n\
             struct S { void M() {} }\n\
             record R(int X) { void M() {} }\n\
             class Impl : IRepo { void IRepo.FindAll() {} }\n",
        );
        assert_eq!(vis_of(&f, "C.M"), Vis::Private);
        assert_eq!(vis_of(&f, "S.M"), Vis::Private);
        assert_eq!(vis_of(&f, "R.M"), Vis::Private);
        // An explicit interface implementation carries no modifier and
        // keeps its old reading here; dispatch to it is FINDINGS B2.
        assert_eq!(vis_of(&f, "Impl.FindAll"), Vis::Private);
    }

    #[test]
    fn an_accessor_takes_its_property_accessibility_unless_it_has_its_own() {
        use cgg_core::Vis;
        let f = extract(
            "class C {\n  public int P { get; private set; }\n  int Q { get; set; }\n}\n",
        );
        let acc = |name: &str, line: u32| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == name && d.start_line == line)
                .unwrap_or_else(|| panic!("{name}@{line}"))
                .vis
        };
        assert_eq!(acc("get", 2), Vis::Public);
        assert_eq!(acc("set", 2), Vis::Private);
        assert_eq!(acc("get", 3), Vis::Private);
    }
}
