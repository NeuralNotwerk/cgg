//! Kivy KV plugin — event bindings as callables, Python-like calls as refs.
//!
//! `.kv` files are markup: a rule `<FacingWizardPopup>:` (or a root widget)
//! describes a widget tree, and `on_release: root.generate_program()` is a
//! real call from that binding into Python. The grammar (vendored
//! `tree-sitter-kivy`) treats expressions as a flat mix of identifiers,
//! dots and parenthesized argument lists, so a call is `ident (. ident)* ( )`.
//!
//! Each property whose expression (or indented event-suite body)
//! contains at least one call becomes a definition
//! (`FacingWizardPopup.on_release:42`) spanning that property.
//! `app.root.foo()` is a linker sentinel for the widget `App.build()`
//! returns. The kv→python linker in `cgg-resolve::ffi` binds the refs
//! to Python methods. See `vendor/kivy/PROVENANCE.md`.

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord, Vis, ids::FileId};
use tree_sitter::{Node, Tree};
use tree_sitter_language::LanguageFn;

use crate::LanguagePlugin;

unsafe extern "C" {
    fn tree_sitter_kivy() -> *const ();
}
/// Raw binding to the vendored Kivy grammar's C entry point.
const KIVY_LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_kivy) };

/// Python/KV names that look like calls (`dp(12)`, `str(x)`) but are not
/// user methods we should try to resolve into the Python graph.
const SKIP_CALLEES: &[&str] = &[
    "dp",
    "sp",
    "pt",
    "mm",
    "cm",
    "inch",
    "rgb",
    "rgba",
    "str",
    "int",
    "float",
    "len",
    "min",
    "max",
    "abs",
    "bool",
    "list",
    "dict",
    "tuple",
    "range",
    "enumerate",
    "zip",
    "print",
    "super",
    "property",
    "hex",
    "ord",
    "chr",
    "round",
    "sum",
    "sorted",
    "reversed",
    "any",
    "all",
];

#[derive(Debug)]
pub struct KivyPlugin;

impl LanguagePlugin for KivyPlugin {
    fn id(&self) -> &'static str {
        "kivy"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".kv"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        KIVY_LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "kivy");
        let file_class = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| s.starts_with(|c: char| c.is_ascii_uppercase() || c == '_'))
            .unwrap_or("")
            .to_string();
        let mut w = KivyWalker {
            source,
            facts: &mut facts,
            rule_stack: Vec::new(),
            widget_stack: Vec::new(),
            file_class,
            root_widget: String::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct KivyWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    /// Enclosing `<ClassName>:` rules, innermost last.
    rule_stack: Vec<String>,
    /// Enclosing widget class names (`Button`, `BoxLayout`, …).
    widget_stack: Vec<String>,
    file_class: String,
    /// First top-level widget, used when there is no `<Rule>`.
    root_widget: String,
}

impl<'a> KivyWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn owner_class(&self) -> String {
        if let Some(r) = self.rule_stack.last() {
            return r.clone();
        }
        if !self.file_class.is_empty() {
            return self.file_class.clone();
        }
        self.root_widget.clone()
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "rule" => {
                if let Some(sel) = first_named_child_of_kind(node, "selector")
                    && let Some(name_n) = sel.child_by_field_name("name")
                {
                    let name = self.text(name_n).to_string();
                    if !name.is_empty() {
                        self.rule_stack.push(name);
                        self.walk_children(node);
                        self.rule_stack.pop();
                        return;
                    }
                }
            }
            "widget" => {
                if let Some(name_n) = node.child_by_field_name("name") {
                    let name = self.text(name_n).to_string();
                    // `if cond: foo()` / `else: bar()` are Python
                    // statements. The grammar's `:` is also a widget
                    // delimiter, so they show up as widgets named `if`
                    // / `else`. Harvest their calls; do not push them
                    // as owners.
                    if is_python_keyword(&name) {
                        self.record_anonymous_suite(node);
                        return;
                    }
                    if !name.is_empty() {
                        if self.widget_stack.is_empty()
                            && self.rule_stack.is_empty()
                            && self.root_widget.is_empty()
                        {
                            self.root_widget = name.clone();
                        }
                        self.widget_stack.push(name);
                        self.walk_children(node);
                        self.widget_stack.pop();
                        return;
                    }
                }
            }
            "property" => {
                self.record_property(node);
                return;
            }
            "ERROR" => {
                // Parse recovery ejects `if cond: app.root.play()` to a
                // root-level ERROR in large KV files. Scan it as a
                // binding suite so the calls still become refs.
                self.record_anonymous_suite(node);
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

    fn record_property(&mut self, node: Node) {
        let Some(inner) = first_named_child(node) else {
            return;
        };
        let Some(name_n) = inner.child_by_field_name("name") else {
            return;
        };
        let prop = self.text(name_n).trim().to_string();
        if prop.is_empty() {
            return;
        }
        let mut calls = collect_property_calls(inner, self.source);
        // `on_release: if cond: app.root.play()` is one Python line, but
        // the grammar's `:` is also the property delimiter, so `play()`
        // lands in following sibling ERROR/expression nodes. Scan those
        // too and attach them to this binding. The def span has to cover
        // the leftover so the kv→python linker still sees the refs as
        // enclosed by this binding.
        let (start, end) = property_scan_span(node);
        for extra in scan_text_calls(self.source, start, end) {
            if !calls
                .iter()
                .any(|c| c.byte == extra.byte && c.name == extra.name)
            {
                calls.push(extra);
            }
        }
        if calls.is_empty() {
            return;
        }

        let owner = self.owner_class();
        let owner = if owner.is_empty() {
            "kv".to_string()
        } else {
            owner
        };
        let sl = (node.start_position().row as u32) + 1;
        let end_line = line_at(self.source, end.saturating_sub(1).max(start));
        let el = end_line.max((node.end_position().row as u32) + 1);
        let qn = format!("{owner}.{prop}:{sl}");
        self.facts.definitions.push(DefRecord {
            simple_name: prop.clone(),
            qualified_name: qn,
            variant: DefVariant::InherentMethod,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: end as u32,
            signature_hint: format!("{prop}: …"),
            visibility: String::new(),
            vis: Vis::Public,
            attributes: vec!["kv-binding".into()],
            ..Default::default()
        });

        for call in calls {
            let preferred =
                preferred_owner(&call.receiver, &owner, self.widget_stack.last());
            self.facts.references.push(RefRecord {
                name: call.name,
                receiver_hint: call.receiver,
                site_line: call.line,
                site_byte: call.byte,
                context: preferred,
                ..Default::default()
            });
        }
    }

    /// A parse-error node or a keyword "widget" (`if`/`else`) holding
    /// Python statements the grammar could not attach to a property.
    fn record_anonymous_suite(&mut self, node: Node) {
        let mut calls = scan_text_calls(self.source, node.start_byte(), node.end_byte());
        calls.retain(|c| {
            !self
                .facts
                .references
                .iter()
                .any(|r| r.site_byte == c.byte && r.name == c.name)
        });
        if calls.is_empty() {
            return;
        }
        let owner = self.owner_class();
        let owner = if owner.is_empty() {
            "kv".to_string()
        } else {
            owner
        };
        let sl = (node.start_position().row as u32) + 1;
        let el = (node.end_position().row as u32) + 1;
        self.facts.definitions.push(DefRecord {
            simple_name: "kv_suite".into(),
            qualified_name: format!("{owner}.kv_suite:{sl}"),
            variant: DefVariant::InherentMethod,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: "…".into(),
            visibility: String::new(),
            vis: Vis::Public,
            attributes: vec!["kv-binding".into()],
            ..Default::default()
        });
        for call in calls {
            let preferred =
                preferred_owner(&call.receiver, &owner, self.widget_stack.last());
            self.facts.references.push(RefRecord {
                name: call.name,
                receiver_hint: call.receiver,
                site_line: call.line,
                site_byte: call.byte,
                context: preferred,
                ..Default::default()
            });
        }
    }
}

struct KvCall {
    name: String,
    receiver: String,
    line: u32,
    byte: u32,
}

fn preferred_owner(receiver: &str, rule: &str, widget: Option<&String>) -> String {
    let recv = receiver.trim();
    if recv.is_empty() || recv == "root" {
        return rule.to_string();
    }
    if recv == "self" {
        return widget.cloned().unwrap_or_else(|| rule.to_string());
    }
    if recv == "app" {
        return "App".to_string();
    }
    // `app.root.controller.estopCommand` → Controller (last real segment).
    // `app.root.foo()` has no such segment; the linker maps the `app.root`
    // sentinel onto the widget class `App.build()` returns.
    recv.rsplit('.')
        .find(|s| !matches!(*s, "app" | "root" | "self" | "ids" | "parent"))
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .unwrap_or_else(|| {
            if recv == "app.root" || recv.starts_with("app.") {
                "app.root".to_string()
            } else {
                String::new()
            }
        })
}

fn collect_property_calls(inner: Node, source: &[u8]) -> Vec<KvCall> {
    let ast_root = inner
        .child_by_field_name("value")
        .or_else(|| inner.child_by_field_name("body"));
    let mut calls = ast_root
        .map(|n| collect_calls(n, source))
        .unwrap_or_default();
    let scan_node = ast_root.unwrap_or(inner);
    for extra in scan_text_calls(source, scan_node.start_byte(), scan_node.end_byte()) {
        if !calls
            .iter()
            .any(|c| c.byte == extra.byte && c.name == extra.name)
        {
            calls.push(extra);
        }
    }
    calls
}

fn collect_calls(node: Node, source: &[u8]) -> Vec<KvCall> {
    let mut out = Vec::new();
    collect_calls_into(node, source, &mut out);
    out
}

fn collect_calls_into(node: Node, source: &[u8], out: &mut Vec<KvCall>) {
    if node.kind() == "expression" {
        scan_expression_parts(node, source, out);
        return;
    }
    walk_call_children(node, source, out);
}

fn walk_call_children(node: Node, source: &[u8], out: &mut Vec<KvCall>) {
    let mut c = node.walk();
    if c.goto_first_child() {
        loop {
            collect_calls_into(c.node(), source, out);
            if !c.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Walk a flat KV `expression` (idents, `.`, `(…)` ) and emit calls.
fn scan_expression_parts(node: Node, source: &[u8], out: &mut Vec<KvCall>) {
    let mut path: Vec<(String, u32, u32)> = Vec::new();
    let mut last_was_dot = false;
    let mut c = node.walk();
    if !c.goto_first_child() {
        return;
    }
    loop {
        let n = c.node();
        match n.kind() {
            "identifier" => {
                let text = n.utf8_text(source).unwrap_or("").to_string();
                if is_python_keyword(&text) {
                    path.clear();
                    last_was_dot = false;
                } else {
                    let line = (n.start_position().row as u32) + 1;
                    let byte = n.start_byte() as u32;
                    if last_was_dot && !path.is_empty() {
                        path.push((text, line, byte));
                    } else {
                        path.clear();
                        path.push((text, line, byte));
                    }
                    last_was_dot = false;
                }
            }
            "punctuation" if matches!(n.utf8_text(source).unwrap_or(""), "." | ":") => {
                // `.` continues a path; `:` is a Python suite delimiter
                // (`if cond: call()`) and starts a new statement.
                if n.utf8_text(source).unwrap_or("") == "." {
                    last_was_dot = true;
                } else {
                    path.clear();
                    last_was_dot = false;
                }
            }
            "parenthesized_expression" => {
                if let Some((name, line, byte)) = path.pop()
                    && !name.is_empty()
                    && !SKIP_CALLEES.contains(&name.as_str())
                    && !is_python_keyword(&name)
                {
                    let receiver = path
                        .iter()
                        .map(|(s, _, _)| s.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    out.push(KvCall {
                        name,
                        receiver,
                        line,
                        byte,
                    });
                }
                path.clear();
                last_was_dot = false;
                collect_calls_into(n, source, out);
            }
            "list_expression" | "dictionary_expression" | "expression" => {
                path.clear();
                last_was_dot = false;
                collect_calls_into(n, source, out);
            }
            _ => {
                path.clear();
                last_was_dot = false;
            }
        }
        if !c.goto_next_sibling() {
            break;
        }
    }
}

/// Byte-scan a KV `property_block` body. Indented `on_release:` suites
/// are Python statements (`root.apply_changes()`), which the grammar
/// classifies as a widget/property block rather than an expression, so
/// the AST walk above sees no `expression` nodes to harvest.
fn scan_text_calls(source: &[u8], start: usize, end: usize) -> Vec<KvCall> {
    let end = end.min(source.len());
    let start = start.min(end);
    let bytes = &source[start..end];
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'"' || b == b'\'' {
            i = skip_quoted(bytes, i);
            continue;
        }
        if is_ident_start(b) {
            let mut parts: Vec<(String, usize)> = Vec::new();
            loop {
                let (name, next) = read_ident(bytes, i);
                parts.push((name, i));
                i = skip_ws(bytes, next);
                if i < bytes.len() && bytes[i] == b'.' {
                    i = skip_ws(bytes, i + 1);
                    continue;
                }
                break;
            }
            i = skip_ws(bytes, i);
            if i < bytes.len() && bytes[i] == b'(' {
                if let Some((name, off)) = parts.pop()
                    && !name.is_empty()
                    && !SKIP_CALLEES.contains(&name.as_str())
                    && !is_python_keyword(&name)
                {
                    let abs = start + off;
                    out.push(KvCall {
                        name,
                        receiver: parts
                            .iter()
                            .map(|(s, _)| s.as_str())
                            .collect::<Vec<_>>()
                            .join("."),
                        line: line_at(source, abs),
                        byte: abs as u32,
                    });
                }
                let args_from = i + 1;
                let args_to = matching_paren(bytes, i);
                out.extend(scan_text_calls(
                    source,
                    start + args_from,
                    start + args_to.min(bytes.len()),
                ));
                i = args_to.saturating_add(1);
                continue;
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Bytes of this property plus following leftover siblings.
///
/// Covers the grammar split `on_release: if x: foo()` where the second
/// `:` ends the property node before `foo()`. Only ERROR / expression
/// leftovers are included; the next real widget or property stops the
/// span so an earlier `text:` cannot swallow later event bindings.
fn property_scan_span(node: Node) -> (usize, usize) {
    let start = node.start_byte();
    let mut end = node.end_byte();
    let Some(parent) = node.parent() else {
        return (start, end);
    };
    let mut past = false;
    let mut c = parent.walk();
    if !c.goto_first_child() {
        return (start, end);
    }
    loop {
        let n = c.node();
        if n.start_byte() == node.start_byte()
            && n.end_byte() == node.end_byte()
            && n.kind() == node.kind()
        {
            past = true;
        } else if past {
            if !is_property_continuation(n) {
                break;
            }
            if n.end_byte() > end {
                end = n.end_byte();
            }
        }
        if !c.goto_next_sibling() {
            break;
        }
    }
    (start, end)
}

fn is_property_continuation(n: Node) -> bool {
    match n.kind() {
        "ERROR"
        | "expression"
        | "identifier"
        | "punctuation"
        | "comment"
        | "parenthesized_expression"
        | "list_expression"
        | "dictionary_expression" => true,
        _ => !n.is_named(),
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn read_ident(bytes: &[u8], start: usize) -> (String, usize) {
    let mut i = start;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    (String::from_utf8_lossy(&bytes[start..i]).into_owned(), i)
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    i
}

fn skip_quoted(bytes: &[u8], mut i: usize) -> usize {
    let q = bytes[i];
    i += 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i = i.saturating_add(2);
            continue;
        }
        i += 1;
        if b == q {
            break;
        }
    }
    i
}

fn matching_paren(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            i = skip_quoted(bytes, i);
            continue;
        }
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    bytes.len()
}

fn line_at(source: &[u8], byte: usize) -> u32 {
    1 + source[..byte.min(source.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32
}

fn is_python_keyword(s: &str) -> bool {
    matches!(
        s,
        "if" | "else"
            | "elif"
            | "and"
            | "or"
            | "not"
            | "in"
            | "is"
            | "lambda"
            | "for"
            | "while"
            | "with"
            | "as"
            | "True"
            | "False"
            | "None"
            | "return"
            | "yield"
            | "pass"
            | "class"
            | "def"
    )
}

fn first_named_child(node: Node) -> Option<Node> {
    let mut c = node.walk();
    if c.goto_first_child() {
        loop {
            if c.node().is_named() {
                return Some(c.node());
            }
            if !c.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn first_named_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut c = node.walk();
    if c.goto_first_child() {
        loop {
            if c.node().kind() == kind {
                return Some(c.node());
            }
            if !c.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str, path: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&KIVY_LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        KivyPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from(path),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn plugin_loads() {
        let plugin = KivyPlugin;
        assert_eq!(plugin.id(), "kivy");
        assert!(plugin.extensions().contains(&".kv"));
        let _ = plugin.ts_language();
    }

    #[test]
    fn extracts_root_method_call_from_rule() {
        let src = "\
<FacingWizardPopup>:
    Button:
        text: \"Go\"
        on_release: root.generate_program()
";
        let f = extract(src, "/tmp/FacingWizardPopup.kv");
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name.contains("FacingWizardPopup.on_release")),
            "defs: {:?}",
            f.definitions
        );
        let r = f
            .references
            .iter()
            .find(|r| r.name == "generate_program")
            .expect("generate_program ref");
        assert_eq!(r.receiver_hint, "root");
        assert_eq!(r.context, "FacingWizardPopup");
    }

    #[test]
    fn extracts_ternary_with_two_calls() {
        let src = "\
<Preview>:
    Button:
        on_release: app.root.controller.estopCommand() if app.state == 'Run' else root.dismiss()
";
        let f = extract(src, "/tmp/Preview.kv");
        let names: Vec<_> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"estopCommand"), "refs: {:?}", f.references);
        assert!(names.contains(&"dismiss"), "refs: {:?}", f.references);
        let estop = f
            .references
            .iter()
            .find(|r| r.name == "estopCommand")
            .unwrap();
        assert_eq!(estop.receiver_hint, "app.root.controller");
        assert_eq!(estop.context, "Controller");
    }

    #[test]
    fn skips_metrics_helpers() {
        let src = "\
<StatusCard>:
    padding: dp(12)
    Label:
        text: \"hi\"
";
        let f = extract(src, "/tmp/StatusCard.kv");
        assert!(
            f.references.iter().all(|r| r.name != "dp"),
            "dp should not be a ref: {:?}",
            f.references
        );
        assert!(f.definitions.is_empty(), "no call → no binding def");
    }

    #[test]
    fn app_root_receiver_is_a_linker_sentinel() {
        let src = "\
<StatusBar>:
    Button:
        on_release: app.root.run_macro(1)
";
        let f = extract(src, "/tmp/StatusBar.kv");
        let r = f
            .references
            .iter()
            .find(|r| r.name == "run_macro")
            .expect("run_macro ref");
        assert_eq!(r.receiver_hint, "app.root");
        assert_eq!(r.context, "app.root");
    }

    #[test]
    fn indented_on_release_suite_extracts_each_call() {
        let src = "\
<WCSSettingsPopup>:
    Button:
        on_release:
            root.apply_changes()
            root.dismiss()
";
        let f = extract(src, "/tmp/WCSSettingsPopup.kv");
        let names: Vec<_> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.contains(&"apply_changes"),
            "apply_changes missing from indented suite: {:?}",
            f.references
        );
        assert!(
            names.contains(&"dismiss"),
            "dismiss missing from indented suite: {:?}",
            f.references
        );
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name.contains("WCSSettingsPopup.on_release")),
            "defs: {:?}",
            f.definitions
        );
        let apply = f
            .references
            .iter()
            .find(|r| r.name == "apply_changes")
            .unwrap();
        assert_eq!(apply.context, "WCSSettingsPopup");
    }

    #[test]
    fn statement_if_extracts_the_call_after_the_colon() {
        let src = "\
<Makera>:
    Button:
        on_release: if root.mode == 'Run': app.root.play(app.selected_remote_filename)
";
        let f = extract(src, "/tmp/makera.kv");
        let play = f
            .references
            .iter()
            .find(|r| r.name == "play")
            .unwrap_or_else(|| {
                panic!("play missing from statement-if: {:?}", f.references)
            });
        assert_eq!(play.receiver_hint, "app.root");
        assert_eq!(play.context, "app.root");
    }

    #[test]
    fn indented_if_else_suite_extracts_both_calls() {
        let src = "\
<Makera>:
    Button:
        on_release:
            root.dismiss()
            if root.mode == 'Run': app.root.play(app.selected_remote_filename)
            else: app.root.apply()
";
        let f = extract(src, "/tmp/makera.kv");
        let names: Vec<_> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.contains(&"dismiss"),
            "dismiss missing from if/else suite: {:?}",
            f.references
        );
        assert!(
            names.contains(&"play"),
            "play missing from if/else suite: {:?}",
            f.references
        );
        assert!(
            names.contains(&"apply"),
            "apply missing from if/else suite: {:?}",
            f.references
        );
        let play = f.references.iter().find(|r| r.name == "play").unwrap();
        assert_eq!(play.receiver_hint, "app.root");
        let apply = f.references.iter().find(|r| r.name == "apply").unwrap();
        assert_eq!(apply.receiver_hint, "app.root");
    }

    #[test]
    fn nested_ternary_in_play_args_still_extracts_play_and_apply() {
        // Exact shape from carveracontroller/makera.kv (the Run/Do button).
        let src = "\
<PreviewPopup>:
    BoxLayout:
        Button:
            text: tr._('Run') if root.mode == 'Run' else tr._('Do ') + root.mode
            on_release:
                root.dismiss()
                if root.mode == 'Run': app.root.play(app.selected_remote_filename, txt_startline.text if cbx_startline.active else None)
                else: app.root.apply()
                app.root.content.transition.direction = 'right'
                app.root.content.current = 'File'
                app.root.cmd_manager.current = 'gcode_cmd_page'
";
        let f = extract(src, "/tmp/makera.kv");
        let names: Vec<_> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.contains(&"play"),
            "play missing from nested-ternary suite: {:?}",
            f.references
        );
        assert!(
            names.contains(&"apply"),
            "apply missing from nested-ternary suite: {:?}",
            f.references
        );
        let play = f.references.iter().find(|r| r.name == "play").unwrap();
        assert_eq!(play.receiver_hint, "app.root");
        assert_eq!(play.context, "app.root");
    }

    #[test]
    fn error_node_if_suite_extracts_play() {
        // Bare statement-if is recovered as ERROR, which is how large
        // KV files (makera.kv) lose `app.root.play()` from the widget
        // tree. The ERROR walker must still harvest the call.
        let src = "\
if root.mode == 'Run': app.root.play(1)
else: app.root.apply()
";
        let f = extract(src, "/tmp/makera.kv");
        let play = f
            .references
            .iter()
            .find(|r| r.name == "play")
            .unwrap_or_else(|| {
                panic!("play missing from ERROR suite: {:?}", f.references)
            });
        assert_eq!(play.receiver_hint, "app.root");
        assert_eq!(play.context, "app.root");
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "apply" && r.receiver_hint == "app.root"),
            "apply missing from ERROR suite: {:?}",
            f.references
        );
    }

    #[test]
    fn filename_class_when_no_rule() {
        let src = "\
BoxLayout:
    Button:
        on_release: root.open_popup()
";
        let f = extract(src, "/tmp/SettingSnippet.kv");
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name.starts_with("SettingSnippet.on_release")),
            "defs: {:?}",
            f.definitions
        );
    }
}
