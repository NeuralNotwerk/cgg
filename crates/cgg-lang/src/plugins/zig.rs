//! Zig plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ZigPlugin;

impl LanguagePlugin for ZigPlugin {
    fn id(&self) -> &'static str {
        "zig"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".zig"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_zig::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "zig");
        let mut w = ZigWalker {
            source,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        // A build script names the modules its packages are imported as:
        // `b.addModule("pkg", .{ .root_source_file = b.path("src/main.zig") })`
        // makes `@import("pkg")` that file (WS14).
        if path.file_name().is_some_and(|n| n == "build.zig") {
            facts
                .imports
                .extend(super::manifest::zig_build_modules(source));
        }
        facts
    }
}

/// `@import("x").a.b` → `("x", "a.b")`: a path of plain members off an
/// import.
fn import_member(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("@import(")?;
    let close = rest.find(')')?;
    let spec = rest[..close].trim().trim_matches('"');
    let members = rest[close + 1..].strip_prefix('.')?;
    let plain = !spec.is_empty()
        && members.split('.').all(|seg| {
            !seg.is_empty() && seg.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
        });
    plain.then_some((spec, members))
}

struct ZigWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> ZigWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
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
                        ..Default::default()
                    });
                }
                self.walk_children(node);
                return;
            }
            // `const addRunArtifact = link.addRunArtifact;` at file level:
            // the name is that declaration of the aliased container, which
            // the resolver follows (`const link = @import("link.zig")`).
            "variable_declaration" => {
                if node.parent().is_some_and(|p| p.kind() == "source_file") {
                    self.record_alias(node);
                }
                self.walk_children(node);
                return;
            }
            "call_expression" => {
                let func = node
                    .child_by_field_name("function")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !func.is_empty() {
                    let (name, recv) = if let Some(pos) = func.rfind('.') {
                        (func[pos + 1..].to_string(), func[..pos].to_string())
                    } else {
                        (func, String::new())
                    };
                    self.facts.references.push(RefRecord {
                        name,
                        receiver_hint: recv,
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                        ..Default::default()
                    });
                }
                self.walk_children(node);
                return;
            }
            "builtin_function" => {
                // @import("std") -> import
                let bi = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "builtin_identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if bi == "@import" {
                    let arg = node
                        .children(&mut node.walk())
                        .find(|c| c.kind() == "arguments")
                        .and_then(|a| {
                            a.children(&mut a.walk()).find(|c| c.kind() == "string")
                        })
                        .map(|n| self.text(n).trim_matches('"').to_string())
                        .unwrap_or_default();
                    if !arg.is_empty() {
                        // `const NAME = @import("x");`: the name it is
                        // bound under in this file.
                        let alias = node
                            .parent()
                            .filter(|p| p.kind() == "variable_declaration")
                            .and_then(|p| {
                                p.children(&mut p.walk())
                                    .find(|c| c.kind() == "identifier")
                                    .map(|n| self.text(n).to_string())
                            })
                            .unwrap_or_default();
                        self.facts.imports.push(ImportRecord {
                            kind: "import".into(),
                            path: arg,
                            alias,
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn record_alias(&mut self, node: Node) {
        let mut c = node.walk();
        let children: Vec<Node> = node.named_children(&mut c).collect();
        let Some(name) = children.iter().find(|n| n.kind() == "identifier") else {
            return;
        };
        let Some(value) = children.iter().find(|n| n.kind() == "field_expression") else {
            return;
        };
        let path = self.text(*value).trim();
        // `const checksum = @import("vsr/checksum.zig").checksum;`: a
        // member of an imported file, recorded as `<file>#<members>`.
        if let Some((spec, members)) = import_member(path) {
            self.facts.imports.push(ImportRecord {
                kind: "import-member".into(),
                path: format!("{spec}#{members}"),
                alias: self.text(*name).to_string(),
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
            return;
        }
        let plain = path.split('.').count() >= 2
            && path.split('.').all(|seg| {
                !seg.is_empty() && seg.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
            });
        if !plain {
            return;
        }
        self.facts.imports.push(ImportRecord {
            kind: "alias".into(),
            path: path.to_string(),
            alias: self.text(*name).to_string(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_zig::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ZigPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.zig"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn functions_extracted() {
        let f = extract(
            "fn add(a: i32, b: i32) i32 { return a + b; }\npub fn main() !void {}\n",
        );
        assert!(f.definitions.iter().any(|d| d.simple_name == "add"));
        assert!(f.definitions.iter().any(|d| d.simple_name == "main"));
    }

    /// FINDINGS R13: `const addRunArtifact = link.addRunArtifact;` is a
    /// file-local alias of another file's function; a value that is not
    /// a plain path is not one.
    #[test]
    fn a_member_alias_is_recorded() {
        let f = extract(
            "const link = @import(\"link.zig\");\n\
             const addRunArtifact = link.addRunArtifact;\n\
             const n = link.items[0];\n",
        );
        let aliases: Vec<(&str, &str)> = f
            .imports
            .iter()
            .filter(|i| i.kind == "alias")
            .map(|i| (i.path.as_str(), i.alias.as_str()))
            .collect();
        assert_eq!(aliases, vec![("link.addRunArtifact", "addRunArtifact")]);
    }

    #[test]
    fn calls_and_imports() {
        let f = extract(
            "const std = @import(\"std\");\nfn f() void { std.debug.print(\"hi\", .{}); add(1,2); }\n",
        );
        assert!(f.imports.iter().any(|i| i.path == "std"));
        assert!(f.references.iter().any(|r| r.name == "add"));
    }

    /// WS14: an import carries the name it is bound to, and a build
    /// script's `addModule` names a module's root file.
    #[test]
    fn import_names_and_build_modules() {
        let f = extract("const posix = @import(\"os/posix.zig\");\n");
        assert!(f.imports.iter().any(|i| i.kind == "import"
            && i.path == "os/posix.zig"
            && i.alias == "posix"));
        let src = "pub fn build(b: *std.Build) void {\n    _ = b.addModule(\"xev\", .{\n        \
                   .root_source_file = b.path(\"src/main.zig\"),\n    });\n}\n";
        let f = extract("pub const checksum = @import(\"vsr/checksum.zig\").checksum;\n");
        assert!(f.imports.iter().any(|i| i.kind == "import-member"
            && i.path == "vsr/checksum.zig#checksum"
            && i.alias == "checksum"));
        let mods = super::super::manifest::zig_build_modules(src.as_bytes());
        assert_eq!(mods.len(), 1);
        assert_eq!(
            (mods[0].alias.as_str(), mods[0].path.as_str()),
            ("xev", "src/main.zig")
        );
    }
}
