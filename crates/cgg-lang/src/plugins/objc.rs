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
    /// Blanks the export and calling-convention macros the grammar cannot
    /// read (see [`super::c_family::parse_rewrite`]).
    fn parse_source(&self, _path: &Path, source: &[u8]) -> Option<Vec<u8>> {
        super::c_family::parse_rewrite(source, false)
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_objc::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "objc");
        let macro_methods = macro_methods(source);
        let macro_spans: Vec<(u32, u32)> = macro_methods
            .iter()
            .map(|d| (d.start_byte, d.end_byte))
            .collect();
        facts.definitions.extend(macro_methods);
        let mut w = ObjcWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
            reference_sites: ctx.reference_sites,
            macro_spans,
        };
        w.walk(tree.root_node());
        // C function names in value positions (`sort_with(xs, compare)`), read
        // as the C plugin reads them (WS26).
        if ctx.reference_sites {
            let got = super::c_values::collect(ctx, tree, source, false, false);
            facts.references.extend(got.refs);
            facts.data_members = got.members;
            facts.type_names = got.types;
        }
        super::preproc::mark(tree.root_node(), &mut facts.definitions);
        facts
    }
}

struct ObjcWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Record selector values (WS26); see `crate::ExtractCtx`.
    reference_sites: bool,
    /// Byte spans of the methods [`macro_methods`] read. The grammar's
    /// error recovery inside one can produce a method or function
    /// definition that is not one; nothing is recorded there.
    macro_spans: Vec<(u32, u32)>,
}

impl<'a> ObjcWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn in_macro_method(&self, n: Node) -> bool {
        let b = n.start_byte() as u32;
        self.macro_spans.iter().any(|&(s, e)| s <= b && b < e)
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
                if !self.in_macro_method(node) {
                    self.record_method(node);
                }
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
                    .filter(|_| !self.in_macro_method(node))
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
                    // `NSSelectorFromString(@"tap:")`: the selector named.
                    if self.reference_sites
                        && cgg_core::stdlib::OBJC_SELECTOR_FROM_STRING
                            .contains(&r.name.as_str())
                        && let Some(lit) = node
                            .child_by_field_name("arguments")
                            .and_then(|a| a.named_child(0))
                            .filter(|a| a.kind() == "string_literal")
                    {
                        let sel =
                            self.text(lit).trim_start_matches('@').trim_matches('"');
                        self.push_selector(lit, sel.to_string());
                    }
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
            // `@selector(tap:)`: the method with that selector.
            "selector_expression" if self.reference_sites => {
                let t = self.text(node);
                let inner = t
                    .trim_start_matches("@selector")
                    .trim()
                    .trim_start_matches('(')
                    .trim_end_matches(')')
                    .to_string();
                self.push_selector(node, inner);
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

    /// A selector (`handle:with:`) as the plugin names methods: its first
    /// keyword (`handle`).
    fn push_selector(&mut self, site: Node, selector: String) {
        let name: String = selector
            .split(':')
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: cgg_core::OBJC_SELECTOR_RECEIVER.to_string(),
            site_line: (site.start_position().row as u32) + 1,
            site_byte: site.start_byte() as u32,
            value_ref: true,
            ..Default::default()
        });
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

/// The methods [`cgg_core::stdlib::OBJC_METHOD_DEFINING_MACROS`] define:
/// `RCT_EXPORT_METHOD(sel:(T)a rest:(U)b) { … }` at the start of a line
/// inside an `@implementation`, read from the text because the grammar
/// cannot parse it. Each is a method of that class named by the
/// selector's first keyword, spanning macro and body, with the macro's
/// name as its attribute.
fn macro_methods(source: &[u8]) -> Vec<DefRecord> {
    let text = blank_comments_and_strings(source);
    let mut out = Vec::new();
    if !cgg_core::stdlib::OBJC_METHOD_DEFINING_MACROS
        .iter()
        .any(|(m, _)| text.windows(m.len()).any(|w| w == m.as_bytes()))
    {
        return out;
    }
    // `@implementation Name` … `@end` ranges, in order.
    let mut classes: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while let Some(at) = find(&text, b"@implementation", i) {
        let mut j = at + "@implementation".len();
        while j < text.len() && text[j].is_ascii_whitespace() {
            j += 1;
        }
        let start = j;
        while j < text.len() && (text[j].is_ascii_alphanumeric() || text[j] == b'_') {
            j += 1;
        }
        let name = String::from_utf8_lossy(&text[start..j]).to_string();
        let end = find(&text, b"@end", j).unwrap_or(text.len());
        if !name.is_empty() {
            classes.push((at, end, name));
        }
        i = end.max(j);
    }
    let mut line_start = 0usize;
    while line_start < text.len() {
        let line_end = text[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(text.len(), |p| line_start + p);
        let mut k = line_start;
        while k < line_end && (text[k] == b' ' || text[k] == b'\t') {
            k += 1;
        }
        let next = line_end + 1;
        let Some(&(mac, sel_arg)) = cgg_core::stdlib::OBJC_METHOD_DEFINING_MACROS
            .iter()
            .find(|(m, _)| {
                text[k..].starts_with(m.as_bytes())
                    && !text
                        .get(k + m.len())
                        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            })
        else {
            line_start = next;
            continue;
        };
        let Some(class) = classes.iter().find(|(s, e, _)| *s < k && k < *e) else {
            line_start = next;
            continue;
        };
        let mut j = k + mac.len();
        while j < text.len() && text[j].is_ascii_whitespace() {
            j += 1;
        }
        let Some(close) = (text.get(j) == Some(&b'('))
            .then(|| matching(&text, j))
            .flatten()
        else {
            line_start = next;
            continue;
        };
        let args = String::from_utf8_lossy(&source[j + 1..close]).to_string();
        let mut b = close + 1;
        while b < text.len() && text[b].is_ascii_whitespace() {
            b += 1;
        }
        let Some(body_end) = (text.get(b) == Some(&b'{'))
            .then(|| matching(&text, b))
            .flatten()
        else {
            line_start = next;
            continue;
        };
        let name = split_args(&args)
            .get(sel_arg)
            .map(|sel| sel.split(':').next().unwrap_or("").trim().to_string())
            .filter(|n| {
                !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        if let Some(name) = name {
            let line_of = |byte: usize| {
                source[..byte].iter().filter(|&&c| c == b'\n').count() as u32 + 1
            };
            out.push(DefRecord {
                qualified_name: format!("{}::{name}", class.2),
                simple_name: name,
                variant: DefVariant::InherentMethod,
                start_line: line_of(k),
                end_line: line_of(body_end),
                start_byte: k as u32,
                end_byte: (body_end + 1) as u32,
                signature_hint: String::from_utf8_lossy(&source[k..=close]).to_string(),
                visibility: String::new(),
                attributes: vec![mac.to_string()],
                has_body: true,
                ..Default::default()
            });
        }
        line_start = body_end.max(line_end) + 1;
    }
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// The index of the bracket closing the one at `open` (`(` or `{`), in
/// text whose comments and literals are blanked.
fn matching(text: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match text[open] {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0usize;
    for (i, &b) in text.iter().enumerate().skip(open) {
        if b == o {
            depth += 1;
        } else if b == c {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Split a macro's argument text at top-level commas.
fn split_args(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in args.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(args[start..].trim());
    out
}

/// `source` with every comment and string or character literal replaced
/// by spaces (newlines kept), so bracket matching and line scanning see
/// code only. Offsets are unchanged.
fn blank_comments_and_strings(source: &[u8]) -> Vec<u8> {
    let mut out = source.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for b in &mut out[from..to] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    };
    while i < source.len() {
        match source[i] {
            b'/' if source.get(i + 1) == Some(&b'/') => {
                let end = source[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(source.len(), |p| i + p);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if source.get(i + 1) == Some(&b'*') => {
                let end = find(source, b"*/", i + 2).map_or(source.len(), |p| p + 2);
                blank(&mut out, i, end);
                i = end;
            }
            q @ (b'"' | b'\'') => {
                let mut j = i + 1;
                while j < source.len() && source[j] != q && source[j] != b'\n' {
                    if source[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let end = (j + 1).min(source.len());
                blank(&mut out, i, end);
                i = end;
            }
            _ => i += 1,
        }
    }
    out
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

    fn extract_with_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_objc::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ObjcPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.m"),
            &tree,
            src.as_bytes(),
        )
    }

    /// WS26: `@selector(…)` and `NSSelectorFromString(@"…")` with a literal
    /// name a method (by its first keyword, as methods are named); a C
    /// function passed by name is a value; a non-literal selector is not.
    #[test]
    fn selectors_and_function_values_are_value_references() {
        let f = extract_with_refs(
            "@implementation A\n\
             - (void)go {\n\
             \x20 [b addTarget:self action:@selector(tap:) forControlEvents:1];\n\
             \x20 SEL s = @selector(handle:with:);\n\
             \x20 SEL t = NSSelectorFromString(@\"viaString:\");\n\
             \x20 SEL u = NSSelectorFromString(name);\n\
             \x20 sort_with(x, n, 4, compare);\n\
             }\n\
             @end\n",
        );
        let selectors: Vec<&str> = f
            .references
            .iter()
            .filter(|r| {
                r.value_ref && r.receiver_hint == cgg_core::OBJC_SELECTOR_RECEIVER
            })
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(selectors, vec!["tap", "handle", "viaString"]);
        assert!(
            f.references.iter().any(|r| r.value_ref
                && r.receiver_hint.is_empty()
                && r.name == "compare"),
            "{:?}",
            f.references
        );
        // `ShaderFunctionMTL::Cast(x)`, split by the parse into an
        // assignment of `ShaderFunctionMTL`, is a call, not that value.
        let f = extract_with_refs(
            "void f(void) {\n  d.fn =\n      ShaderFunctionMTL::Cast(*x.y()).z();\n}\n",
        );
        assert!(
            f.references
                .iter()
                .all(|r| !(r.value_ref && r.name == "ShaderFunctionMTL")),
            "{:?}",
            f.references
        );
        // A method's parameter is a variable, not a function of its name.
        let f = extract_with_refs(
            "@implementation A\n- (void)writeSize:(int)size { report(size); }\n@end\n",
        );
        assert!(
            f.references
                .iter()
                .all(|r| !(r.value_ref && r.name == "size")),
            "{:?}",
            f.references
        );
        assert!(
            extract("@implementation A\n- (void)go { SEL s = @selector(tap:); }\n@end\n")
                .references
                .iter()
                .all(|r| !r.value_ref)
        );
    }

    /// React Native's `RCT_EXPORT_*` macros define methods the grammar
    /// cannot parse: each is a method of its class, named by the
    /// selector's first keyword and tagged with the macro, and the calls
    /// in its body have it as their caller.
    #[test]
    fn method_defining_macros_are_methods() {
        let src = "@implementation Cal\n\
                   RCT_EXPORT_MODULE();\n\
                   - (void)helper {}\n\
                   RCT_EXPORT_METHOD(createEvent:(NSString *)name location:(NSString *)loc)\n\
                   {\n  RCTLogInfo(@\"at %@ )\", name);\n  [self helper];\n}\n\
                   RCT_REMAP_METHOD(find,\n                 findWithResolver:(RCTPromiseResolveBlock)resolve\n                 rejecter:(RCTPromiseRejectBlock)reject)\n\
                   {\n  resolve(@[]);\n}\n\
                   RCT_EXPORT_BLOCKING_SYNCHRONOUS_METHOD(getName)\n{\n  return @\"x\";\n}\n\
                   // RCT_EXPORT_METHOD(commented:(id)x) { }\n\
                   @end\n";
        let f = extract(src);
        let m = |qn: &str| {
            f.definitions
                .iter()
                .find(|d| d.qualified_name == qn)
                .unwrap_or_else(|| panic!("{qn}: {:?}", f.definitions))
        };
        let create = m("Cal::createEvent");
        assert_eq!(create.attributes, vec!["RCT_EXPORT_METHOD"]);
        assert!(create.has_body);
        let find = m("Cal::findWithResolver");
        assert_eq!(find.attributes, vec!["RCT_REMAP_METHOD"]);
        assert_eq!(
            m("Cal::getName").attributes,
            vec!["RCT_EXPORT_BLOCKING_SYNCHRONOUS_METHOD"]
        );
        assert!(f.definitions.iter().all(|d| d.simple_name != "commented"));
        // No spurious definition from the grammar's error recovery.
        assert!(
            f.definitions.iter().all(|d| d.simple_name != "location"
                && d.simple_name != "loc"
                && d.simple_name != "name"),
            "{:?}",
            f.definitions
        );
        // `[self helper]` sits inside `createEvent`'s span.
        let helper_call = f
            .references
            .iter()
            .find(|r| r.name == "helper")
            .expect("helper call");
        assert!(
            create.start_byte <= helper_call.site_byte
                && helper_call.site_byte < create.end_byte
        );
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
