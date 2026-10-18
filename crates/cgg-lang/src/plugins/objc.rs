//! Objective-C plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ObjcPlugin;

impl LanguagePlugin for ObjcPlugin {
    fn id(&self) -> &'static str {
        "objc"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".m", ".mm"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_objc::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "objc");
        let mut w = ObjcWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct ObjcWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> ObjcWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, s: &str) -> String {
        if self.scope.is_empty() {
            s.to_string()
        } else {
            format!("{}::{s}", self.scope.join("::"))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_implementation" | "class_interface" => {
                let name = node
                    .child(1) // class name is second child (after @implementation/@interface)
                    .filter(|c| c.kind() == "identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "method_definition" | "method_declaration" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            "function_definition" => {
                // A C function, named as the C plugin names one: through
                // the pointer layers of its return type (`char *f(void)`).
                // A declarator with no function declarator in it is no
                // function: error recovery reads `if (error) { … }` after an
                // `#if` as a definition of `error`.
                let name = node
                    .child_by_field_name("declarator")
                    .filter(|d| declares_a_function(*d))
                    .map(|d| super::c::fn_name_from_declarator(d, self.source))
                    .unwrap_or_default();
                if !name.is_empty() {
                    let (sl, el) = (
                        (node.start_position().row as u32) + 1,
                        (node.end_position().row as u32) + 1,
                    );
                    self.facts.definitions.push(DefRecord {
                        simple_name: name.clone(),
                        qualified_name: name,
                        variant: DefVariant::FreeFunction,
                        start_line: sl,
                        end_line: el,
                        start_byte: node.start_byte() as u32,
                        end_byte: node.end_byte() as u32,
                        signature_hint: super::extract_signature(self.text(node)),
                        visibility: String::new(),
                        attributes: Vec::new(),
                        has_body: true,
                        ..Default::default()
                    });
                }
                self.walk_children(node);
                return;
            }
            // A plain C call (`helper(x)`, `NSLog(@"…")`), read as the C
            // plugin reads one. A call through a field (`self.block(x)`,
            // `s->fp(x)`) is a block or function pointer, never a method.
            "call_expression" => {
                if let Some(r) = super::c::call_ref(node, self.source)
                    .filter(|r| r.receiver_hint.is_empty())
                {
                    self.facts.references.push(r);
                }
                self.walk_children(node);
                return;
            }
            "message_expression" => {
                let method = node
                    .child_by_field_name("method")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let recv_node = node.child_by_field_name("receiver");
                let recv = recv_node
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // `[[Foo alloc] initWithX:1]`: the receiver is a new
                // `Foo`, so the initializer is looked up on `Foo`.
                let constructed = recv_node
                    .filter(|r| r.kind() == "message_expression")
                    .filter(|r| {
                        r.child_by_field_name("method").is_some_and(|m| {
                            cgg_core::stdlib::is_constructing_method("objc", self.text(m))
                        })
                    })
                    .and_then(|r| r.child_by_field_name("receiver"))
                    .filter(|c| c.kind() == "identifier")
                    .map(|c| self.text(c).to_string());
                let typed_receiver = constructed.is_some();
                let recv = constructed.unwrap_or(recv);
                if !method.is_empty() {
                    self.facts.references.push(RefRecord {
                        name: method,
                        receiver_hint: recv,
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                        typed_receiver,
                        ..Default::default()
                    });
                }
                self.walk_children(node);
                return;
            }
            "preproc_include" | "preproc_import" => {
                // Quoted `#import "Foo.h"` is a project include, the same
                // edge the C/C++ plugins record as kind `include`. Angle
                // brackets stay `system-include` and keep their raw text:
                // framework rules match the literal `<UIKit` prefix.
                let Some(p) = node.child_by_field_name("path") else {
                    return;
                };
                let raw = self.text(p).trim_matches('"');
                if raw.is_empty() {
                    return;
                }
                let kind = match p.kind() {
                    "string_literal" | "string_content" => "include",
                    "system_lib_string" => "system-include",
                    _ => "import",
                };
                self.facts.imports.push(ImportRecord {
                    kind: kind.into(),
                    path: raw.to_string(),
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                self.walk(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_method(&mut self, node: Node) {
        // Method selector is the identifier child
        let name = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier" || c.kind() == "keyword_selector")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        // Check if class method (+) or instance method (-)
        let method_text = self.text(node);
        let is_class = method_text.trim_start().starts_with('+');
        let variant = if name == "init" {
            DefVariant::Constructor
        } else if is_class {
            DefVariant::StaticMethod
        } else {
            DefVariant::InherentMethod
        };
        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // A `@protocol` requirement is a declaration its adopters
            // implement.
            attributes: if node
                .parent()
                .is_some_and(|p| p.kind() == "protocol_declaration")
            {
                vec![super::attrs::ABSTRACT_MARKER.to_string()]
            } else {
                Vec::new()
            },
            // `@implementation` methods are bodies. A declaration in an
            // `@interface` stays a prototype so unification can absorb it.
            has_body: node.kind() == "method_definition",
            ..Default::default()
        });
    }
}

/// Whether a definition's declarator, through the pointer, attribute
/// and parenthesis layers a return type wraps around it, holds a
/// `function_declarator`.
fn declares_a_function(decl: Node) -> bool {
    match decl.kind() {
        "function_declarator" => true,
        "pointer_declarator" | "attributed_declarator" | "parenthesized_declarator" => {
            decl.child_by_field_name("declarator")
                .or_else(|| decl.named_children(&mut decl.walk()).last())
                .is_some_and(declares_a_function)
        }
        _ => false,
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
        p.set_language(&tree_sitter_objc::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ObjcPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.m"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn class_methods() {
        let src = "@implementation Service\n- (void)run { [self helper]; }\n+ (instancetype)create { return [[Service alloc] init]; }\n@end\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "run"
            && d.variant == DefVariant::InherentMethod
            && d.has_body));
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "create"
                    && d.variant == DefVariant::StaticMethod)
        );
    }

    #[test]
    fn message_sends() {
        let src = "@implementation C\n- (void)f { [self run]; [Helper create]; }\n@end\n";
        let f = extract(src);
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "run" && r.receiver_hint == "self")
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "create" && r.receiver_hint == "Helper")
        );
    }

    #[test]
    fn import_directive_captured() {
        let src = "#import \"Foo.h\"\n#import <UIKit/UIKit.h>\n";
        let f = extract(src);
        let quoted = f
            .imports
            .iter()
            .find(|i| i.kind == "include")
            .expect("quoted import is an include");
        assert_eq!(quoted.path, "Foo.h");
        let sys = f
            .imports
            .iter()
            .find(|i| i.kind == "system-include")
            .expect("angle-bracket import is a system include");
        assert!(
            sys.path.contains("UIKit"),
            "framework rules match this path: {}",
            sys.path
        );
    }

    #[test]
    fn alloc_init_is_typed_by_the_allocated_class() {
        let f = extract(
            "void f(void) { Box *b = [[Box alloc] initWithX:1]; [[b copy] init]; }",
        );
        let refs: Vec<(&str, &str, bool)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str(), r.typed_receiver))
            .collect();
        assert!(refs.contains(&("initWithX", "Box", true)), "{refs:?}");
        assert!(refs.contains(&("init", "[b copy]", false)), "{refs:?}");
    }

    /// The definitions named `qn` and whether each is tagged `abstract`.
    fn abstract_tags(f: &FileFacts, qn: &str) -> Vec<bool> {
        f.definitions
            .iter()
            .filter(|d| d.qualified_name == qn)
            .map(|d| d.attributes.iter().any(|a| a == "abstract"))
            .collect()
    }

    // WS22 gap 7: a `@protocol` requirement is a declaration; an
    // `@interface` declaration is a prototype, not tagged.
    #[test]
    fn protocol_methods_are_tagged_abstract() {
        let f = extract(
            "@protocol P\n- (int)area;\n@end\n@interface K : NSObject\n- (int)size;\n@end\n",
        );
        assert_eq!(abstract_tags(&f, "area"), vec![true]);
        assert_eq!(abstract_tags(&f, "K::size"), vec![false]);
    }

    // WS22 gap 24: plain C calls are recorded as the C plugin records
    // them, and a pointer-returning function is named by its identifier.
    #[test]
    fn plain_c_calls_and_pointer_returning_functions() {
        let f = extract(
            "static char *label(void) { return 0; }\n@implementation K\n- (void)run { helper(1); NSLog(@\"x\"); self.block(2); [self done]; }\n@end\n",
        );
        assert!(f.definitions.iter().any(|d| d.qualified_name == "label"));
        // Near miss: error recovery's `if (error) { … }` is no function.
        // `int (f)(int)` is the function `f`.
        let g = extract("T (error) { done(); }\nint (f)(int a) { return a; }\n");
        let names: Vec<&str> = g
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(names, vec!["f"]);
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("helper", "")), "{refs:?}");
        assert!(refs.contains(&("NSLog", "")), "{refs:?}");
        assert!(refs.contains(&("done", "self")), "{refs:?}");
        // Near miss: a call through a property is a block, never a method.
        assert!(!refs.iter().any(|(n, _)| *n == "block"), "{refs:?}");
    }
}
