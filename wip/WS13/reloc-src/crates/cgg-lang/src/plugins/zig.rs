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
            containers: Vec::new(),
            this_aliases: Vec::new(),
            fns: Vec::new(),
            fn_stack: Vec::new(),
            pending: Vec::new(),
        };
        w.walk(tree.root_node());
        w.type_locals();
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
    /// Containers being walked, innermost last: `(path, body start)`;
    /// an anonymous one (a generic's `return struct { … }`, a struct
    /// declared in a function) has an empty path.
    containers: Vec<(String, u32)>,
    /// `const Self = @This();`: `(container body start, name)`, with
    /// `u32::MAX` for the file.
    this_aliases: Vec<(u32, String)>,
    fns: Vec<ZigFn>,
    /// Functions being walked (indices into `fns`), innermost last.
    fn_stack: Vec<usize>,
    pending: Vec<PendingLocal>,
}

/// A function of the file, for typing locals from what it returns.
struct ZigFn {
    name: String,
    start: u32,
    /// Body start of the container it is declared in; `None` at file
    /// level.
    container: Option<u32>,
    /// Its return type with optionals, pointers and error unions
    /// peeled (`!?*Surface` → `Surface`), when that is a plain name or
    /// path.
    ret: Option<String>,
    /// Its first parameter's name and peeled type (the receiver of a
    /// method call `x.m()`).
    first_param: Option<(String, String)>,
}

/// A local declaration whose type is read once the file is walked.
struct PendingLocal {
    var: String,
    /// End of the declaration: the type holds from there on.
    at: u32,
    /// The function it is declared in (index into `fns`).
    caller: usize,
    ty: PendingTy,
}

enum PendingTy {
    /// Written (`const x: T = …`, `T{ … }`).
    Known(String),
    /// What a call returns (`self.m() orelse return`, `try m()`).
    Call {
        method: String,
        recv: Option<String>,
    },
}

/// Container declaration kinds.
const CONTAINERS: &[&str] = &[
    "struct_declaration",
    "union_declaration",
    "enum_declaration",
    "opaque_declaration",
];

/// `?*const T`, `!T`, `E!*T` → `T` when `T` is a plain name or path
/// (`font.sprite.Canvas`); `None` for anything else (a slice, a generic
/// instantiation, a builtin type).
fn peel_type(mut n: Node, src: &[u8]) -> Option<String> {
    for _ in 0..8 {
        match n.kind() {
            "nullable_type" | "pointer_type" => {
                n = n.named_child(n.named_child_count().checked_sub(1)? as u32)?;
            }
            "error_union_type" => n = n.child_by_field_name("ok")?,
            "identifier" | "field_expression" => {
                let t = n.utf8_text(src).ok()?.trim();
                let plain = t.split('.').all(|seg| {
                    !seg.is_empty()
                        && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
                });
                return plain.then(|| t.to_string());
            }
            _ => return None,
        }
    }
    None
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
                    let first_param = node
                        .children(&mut node.walk())
                        .find(|c| c.kind() == "parameters")
                        .and_then(|ps| {
                            ps.named_children(&mut ps.walk())
                                .find(|p| p.kind() == "parameter")
                        })
                        .and_then(|p| {
                            let n = p.child_by_field_name("name")?;
                            let t =
                                peel_type(p.child_by_field_name("type")?, self.source)?;
                            Some((self.text(n).to_string(), t))
                        });
                    self.fns.push(ZigFn {
                        name: name.clone(),
                        start: node.start_byte() as u32,
                        container: self.containers.last().map(|(_, s)| *s),
                        ret: node
                            .child_by_field_name("type")
                            .and_then(|t| peel_type(t, self.source)),
                        first_param,
                    });
                    self.fn_stack.push(self.fns.len() - 1);
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
                    self.walk_children(node);
                    self.fn_stack.pop();
                    return;
                }
                self.walk_children(node);
                return;
            }
            k if CONTAINERS.contains(&k) => {
                // A container that is not a named declaration's value
                // (`return struct { … }`): its methods are its own.
                self.facts.type_spans.push(cgg_core::TypeSpan {
                    name: String::new(),
                    start_byte: node.start_byte() as u32,
                    end_byte: node.end_byte() as u32,
                    ..Default::default()
                });
                self.containers
                    .push((String::new(), node.start_byte() as u32));
                self.walk_children(node);
                self.containers.pop();
                return;
            }
            // `const addRunArtifact = link.addRunArtifact;` at file level:
            // the name is that declaration of the aliased container, which
            // the resolver follows (`const link = @import("link.zig")`).
            "variable_declaration" => {
                if node.parent().is_some_and(|p| p.kind() == "source_file") {
                    self.record_alias(node, false);
                }
                if self.declare_type(node) {
                    return;
                }
                if self.in_fn() {
                    // `const IO = @import("io.zig").IO;` in a function: an
                    // alias the resolver follows like a file-level one.
                    self.record_alias(node, true);
                    self.record_local(node);
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

    /// Directly in a function body (not in a container nested in one).
    fn in_fn(&self) -> bool {
        self.fn_stack.last().is_some_and(|&f| {
            let fs = self.fns[f].start;
            self.containers.last().is_none_or(|(_, cs)| *cs < fs)
        })
    }

    /// The declaration's name and value (`const NAME[: T] = VALUE;`).
    fn name_and_value<'t>(&self, node: Node<'t>) -> Option<(String, Node<'t>)> {
        let mut c = node.walk();
        let named: Vec<Node> = node.named_children(&mut c).collect();
        let name = named.iter().find(|n| n.kind() == "identifier")?;
        let ty = node.child_by_field_name("type").map(|t| t.id());
        let value = named
            .iter()
            .rev()
            .find(|n| n.id() != name.id() && Some(n.id()) != ty)?;
        Some((self.text(*name).to_string(), *value))
    }

    /// `const T = struct { … }` and `const Self = @This();` outside any
    /// function: record the type's span and walk its body as a
    /// container. True when `node` was such a declaration (and walked).
    fn declare_type(&mut self, node: Node) -> bool {
        let in_fn = self.in_fn();
        let Some((name, value)) = self.name_and_value(node) else {
            return false;
        };
        let start = value.start_byte() as u32;
        if CONTAINERS.contains(&value.kind()) {
            // Inside a function a named struct is local to it: anonymous
            // here (still the container of its own methods).
            let path = match (in_fn, self.containers.last()) {
                (true, _) => String::new(),
                (false, Some((p, _))) if !p.is_empty() => format!("{p}.{name}"),
                (false, Some(_)) => String::new(),
                (false, None) => name,
            };
            self.facts.type_spans.push(cgg_core::TypeSpan {
                name: path.clone(),
                start_byte: start,
                end_byte: value.end_byte() as u32,
                ..Default::default()
            });
            self.containers.push((path, start));
            self.walk_children(value);
            self.containers.pop();
            return true;
        }
        // `pub const frame = Functions(@This()).frame;` in a container: a
        // declaration that may name a function, or a type a function
        // builds (`const C = Common(Self);`), WS14b.
        if !in_fn
            && let Some((_, cs)) = self.containers.last()
            && matches!(
                value.kind(),
                "field_expression" | "identifier" | "call_expression"
            )
        {
            let cs = *cs;
            let text = self.text(value).to_string();
            if let Some(t) = self
                .facts
                .type_spans
                .iter_mut()
                .rev()
                .find(|t| t.start_byte == cs)
            {
                t.decls.push((name.clone(), text));
            }
        }
        let is_this = value.kind() == "builtin_function"
            && value
                .children(&mut value.walk())
                .any(|c| c.kind() == "builtin_identifier" && self.text(c) == "@This");
        if is_this && !in_fn {
            let (key, path, span) = match self.containers.last() {
                Some((p, s)) => {
                    let end = self
                        .facts
                        .type_spans
                        .iter()
                        .rev()
                        .find(|t| t.start_byte == *s)
                        .map_or(*s, |t| t.end_byte);
                    let path = if p.is_empty() {
                        String::new()
                    } else {
                        format!("{p}.{name}")
                    };
                    (*s, path, (*s, end))
                }
                None => (u32::MAX, name.clone(), (0, self.source.len() as u32)),
            };
            self.this_aliases.push((key, name));
            if !path.is_empty() {
                self.facts.type_spans.push(cgg_core::TypeSpan {
                    name: path,
                    start_byte: span.0,
                    end_byte: span.1,
                    ..Default::default()
                });
            }
            return true;
        }
        false
    }

    /// A local declaration in a function: its type when it is written
    /// (`const x: ?*T = …`, `T{ … }`) or is what a call of this file
    /// returns (`self.m() orelse return`, `try m()`), resolved once every
    /// function is known (`type_locals`).
    fn record_local(&mut self, node: Node) {
        let Some(&caller) = self.fn_stack.last() else {
            return;
        };
        let at = node.end_byte() as u32;
        let Some(var) = node
            .named_children(&mut node.walk())
            .find(|n| n.kind() == "identifier")
            .map(|n| self.text(n).to_string())
        else {
            return;
        };
        if let Some(t) = node
            .child_by_field_name("type")
            .and_then(|t| peel_type(t, self.source))
        {
            self.pending.push(PendingLocal {
                var,
                at,
                caller,
                ty: PendingTy::Known(t),
            });
            return;
        }
        let Some((_, value)) = self.name_and_value(node) else {
            return;
        };
        let mut v = value;
        for _ in 0..4 {
            v = match v.kind() {
                "try_expression" | "null_coercion_expression" | "catch_expression" => {
                    match v.named_child(0) {
                        Some(c) => c,
                        None => return,
                    }
                }
                "binary_expression"
                    if v.child_by_field_name("operator")
                        .is_some_and(|o| o.kind() == "orelse") =>
                {
                    match v.child_by_field_name("left") {
                        Some(c) => c,
                        None => return,
                    }
                }
                _ => break,
            };
        }
        let ty = match v.kind() {
            "struct_initializer" => {
                match v.named_child(0).and_then(|t| peel_type(t, self.source)) {
                    Some(t) => PendingTy::Known(t),
                    None => return,
                }
            }
            "call_expression" => {
                let Some(f) = v.child_by_field_name("function") else {
                    return;
                };
                match f.kind() {
                    "identifier" => PendingTy::Call {
                        method: self.text(f).to_string(),
                        recv: None,
                    },
                    "field_expression" => {
                        let (Some(obj), Some(m)) = (
                            f.child_by_field_name("object"),
                            f.child_by_field_name("member"),
                        ) else {
                            return;
                        };
                        if obj.kind() != "identifier" {
                            return;
                        }
                        PendingTy::Call {
                            method: self.text(m).to_string(),
                            recv: Some(self.text(obj).to_string()),
                        }
                    }
                    _ => return,
                }
            }
            _ => return,
        };
        self.pending.push(PendingLocal {
            var,
            at,
            caller,
            ty,
        });
    }

    /// The container `name` denotes from inside container `within` (its
    /// body start; `None` at file level): an enclosing container by name
    /// or path, or a `@This()` alias of one.
    fn container_named(&self, name: &str, within: Option<u32>) -> Option<u32> {
        if let Some((s, _)) = self
            .this_aliases
            .iter()
            .find(|(s, n)| n == name && (Some(*s) == within || *s == u32::MAX))
        {
            return Some(*s);
        }
        self.facts
            .type_spans
            .iter()
            .find(|t| {
                !t.name.is_empty()
                    && (t.name == name || t.name.rsplit('.').next() == Some(name))
            })
            .map(|t| t.start_byte)
            .filter(|_| {
                self.facts
                    .type_spans
                    .iter()
                    .filter(|t| {
                        !t.name.is_empty()
                            && (t.name == name || t.name.rsplit('.').next() == Some(name))
                    })
                    .count()
                    == 1
            })
    }

    /// A type as written in container `within`, made a path the resolver
    /// can follow from the file: a `@This()` alias is the container's
    /// path (or, at file level, the alias itself, which names the file).
    fn type_from(&self, ty: &str, within: Option<u32>) -> Option<String> {
        let alias = self
            .this_aliases
            .iter()
            .find(|(s, n)| n == ty && (Some(*s) == within || *s == u32::MAX));
        match alias {
            Some((u32::MAX, _)) => Some(ty.to_string()),
            Some((s, _)) => self
                .facts
                .type_spans
                .iter()
                .find(|t| t.start_byte == *s && !t.name.is_empty())
                .map(|t| t.name.clone()),
            None => Some(ty.to_string()),
        }
    }

    /// Type the pending locals: a written type as is; a call by what the
    /// one function of this file it reaches returns — `m()` a function of
    /// the caller's container or the file, `x.m()` a method of the
    /// caller's container when `x` is the caller's receiver parameter
    /// (typed as that container), `T.m()` a function of container `T`.
    fn type_locals(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for p in pending {
            let ty = match p.ty {
                PendingTy::Known(t) => {
                    let within = self.fns.get(p.caller).and_then(|f| f.container);
                    self.type_from(&t, within)
                }
                PendingTy::Call { method, recv } => {
                    let caller = p.caller;
                    let here = self.fns.get(caller).and_then(|f| f.container);
                    let target: Vec<Option<u32>> = match &recv {
                        None => vec![here, None],
                        Some(r) => {
                            let receiver_param =
                                self.fns.get(caller).and_then(|f| f.first_param.as_ref());
                            match receiver_param {
                                Some((pn, pt))
                                    if pn == r
                                        && here.is_some()
                                        && self.container_named(pt, here) == here =>
                                {
                                    vec![here]
                                }
                                _ => match self.container_named(r, here) {
                                    // The file's own `@This()` alias: its
                                    // file-level functions.
                                    Some(u32::MAX) => vec![None],
                                    Some(c) => vec![Some(c)],
                                    // Not this file's (`IO.init()` with
                                    // `IO` imported): typed as the call,
                                    // whose declared return type the
                                    // resolver follows.
                                    None => {
                                        self.facts.local_types.push(
                                            cgg_core::LocalType {
                                                var_name: p.var,
                                                type_name: format!("{r}.{method}()"),
                                                scope_byte: p.at,
                                            },
                                        );
                                        continue;
                                    }
                                },
                            }
                        }
                    };
                    let mut found = None;
                    for c in target {
                        let cands: Vec<&ZigFn> = self
                            .fns
                            .iter()
                            .filter(|f| f.name == method && f.container == c)
                            .collect();
                        if let [one] = cands.as_slice() {
                            found = one
                                .ret
                                .as_deref()
                                .and_then(|t| self.type_from(t, one.container));
                        }
                        if !cands.is_empty() {
                            break;
                        }
                    }
                    found
                }
            };
            if let Some(ty) = ty {
                self.facts.local_types.push(cgg_core::LocalType {
                    var_name: p.var,
                    type_name: ty,
                    scope_byte: p.at,
                });
            }
        }
    }

    /// `local`: a declaration in a function, recorded only when it names
    /// a member of an imported file.
    fn record_alias(&mut self, node: Node, local: bool) {
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
        if local {
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

    /// WS14b: a local's type is what is written, a struct literal's type,
    /// or what a call of this file returns (`self.m() orelse return`,
    /// `try m()`, `T.init()` through `@This()`), optionals and pointers
    /// peeled. A call this file cannot see the target of is the call.
    #[test]
    fn locals_are_typed_from_returns_and_annotations() {
        let src = "const Surface = @import(\"surface.zig\").Surface;\n\
                   const Pool = @This();\n\
                   pub fn init() Pool { return .{}; }\n\
                   pub const Tree = struct {\n\
                   \x20   const Self = @This();\n\
                   \x20   fn active(self: *Self) ?*Surface { return null; }\n\
                   \x20   fn me(self: *Tree) Self { return self.*; }\n\
                   \x20   fn f(self: *Tree) !void {\n\
                   \x20       const a = self.active() orelse return;\n\
                   \x20       const b = try self.me();\n\
                   \x20       var c: ?*const Surface = null;\n\
                   \x20       const d = Surface{ .id = 1 };\n\
                   \x20       const e = other.thing();\n\
                   \x20       const p = Pool.init();\n\
                   \x20   }\n\
                   };\n";
        let f = extract(src);
        let ty = |v: &str| {
            f.local_types
                .iter()
                .find(|l| l.var_name == v)
                .map(|l| l.type_name.clone())
        };
        assert_eq!(ty("a").as_deref(), Some("Surface"), "{:?}", f.local_types);
        assert_eq!(ty("b").as_deref(), Some("Tree"), "{:?}", f.local_types);
        assert_eq!(ty("c").as_deref(), Some("Surface"));
        assert_eq!(ty("d").as_deref(), Some("Surface"));
        // Another file's function: typed as the call, whose declared
        // return type the resolver follows.
        assert_eq!(ty("e").as_deref(), Some("other.thing()"));
        assert_eq!(ty("p").as_deref(), Some("Pool"));
        let names: Vec<&str> = f.type_spans.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Pool", "Tree", "Tree.Self"],
            "{:?}",
            f.type_spans
        );
    }
}
