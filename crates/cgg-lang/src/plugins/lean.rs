//! Lean 4 plugin — callable extraction.
//!
//! Lean is a dependently-typed functional language / proof assistant.
//! This plugin is a best-effort *syntactic* extractor: it pulls top-level declarations
//! (`def`/`theorem`/`lemma`/`abbrev`/`instance`, plus `structure` and
//! `inductive` with their fields/constructors), the `import`/`open`
//! graph, ordinary function application, dot-projection calls, and the
//! lemma references inside tactic blocks — `exact`/`apply`/`simp [..]`,
//! `simp only [..]`, and the `rw [..]`/`rewrite [..]` rewrite lists.
//! Tactic combinators that the grammar parses as bare identifiers
//! (`only`, `with`, `at`, ...) are filtered so they are not mistaken for
//! lemma references.
//!
//! Declaration attributes are captured onto each callable: inline
//! `@[simp]`/`@[ext]`/... entries (stored in source form, e.g. `@[simp]`)
//! and an `instance` marker for typeclass instances. These flag decls
//! that are used *implicitly* — a `@[simp]` lemma fired by a bare `simp`,
//! an `instance` chosen by typeclass resolution — so a downstream
//! dead-code pass can avoid reporting them as orphans despite having no
//! by-name call edge.
//!
//! What it deliberately does *not* recover — because it needs the Lean
//! elaborator/kernel, not just surface syntax — is typeclass-method
//! dispatch (`a + b` -> the chosen `HAdd` instance), macro/`notation`
//! expansions, and type-directed dot resolution. Those call sites land
//! in the audit log as unresolved, like every other approximate plugin.
//!
//! The tree-sitter grammar flattens dotted names (`A.B.C`, `Foo.Bar`)
//! into separate `identifier` children, and `namespace`/`section`/`end`
//! are *sibling* marker nodes rather than containers, so the walker
//! maintains an explicit namespace stack and reconstructs dotted groups
//! from the bytes between identifiers (`.` vs whitespace).

use std::path::Path;

use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct LeanPlugin;

impl LanguagePlugin for LeanPlugin {
    fn id(&self) -> &'static str {
        "lean"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".lean"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_lean4::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "lean");
        let mut w = LeanWalker {
            source,
            facts: &mut facts,
            ns_stack: Vec::new(),
            pending_attrs: Vec::new(),
        };
        w.visit(tree.root_node());
        facts
    }
}

fn child_nodes<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.children(&mut c).collect()
}

fn is_ident(kind: &str) -> bool {
    kind == "identifier" || kind == "escaped_identifier"
}

/// Tactic combinators/modifiers the grammar parses as plain identifiers
/// in argument position (`simp only [..]`, `rcases h with ..`,
/// `rw [..] at h`). They name no callable, so they must not be recorded
/// as references when scanning tactic arguments in bare mode.
fn is_tactic_modifier(name: &str) -> bool {
    matches!(
        name,
        "only" | "with" | "at" | "using" | "from" | "generalizing"
    )
}

/// Collapse internal whitespace runs into single spaces.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Signature hint: text up to the body delimiter (`:=` or ` where`) at
/// bracket depth 0, whitespace-collapsed.
fn lean_signature(t: &str) -> String {
    let bytes = t.as_bytes();
    let mut depth = 0i32;
    let mut end = t.len();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = (depth - 1).max(0),
            b':' if depth == 0 && i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                end = i;
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let s = &t[..end];
    let s = match s.find(" where") {
        Some(p) => &s[..p],
        None => s,
    };
    collapse(s)
}

struct LeanWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    /// Active `namespace`/`section` frames. Section frames are empty
    /// strings (they scope variables but do not prefix names).
    ns_stack: Vec<String>,
    /// Attributes contributed by the enclosing `decorated_declaration`
    /// (`@[simp]`, `@[ext]`, ...) plus the `instance` keyword marker. Set
    /// while the wrapped declaration is visited so `push_def` can attach
    /// them, then cleared.
    pending_attrs: Vec<String>,
}

impl<'a> LeanWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn ns_prefix(&self) -> String {
        let parts: Vec<&str> = self
            .ns_stack
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
            .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!("{}.", parts.join("."))
        }
    }

    fn qn(&self, simple: &str) -> String {
        format!("{}{simple}", self.ns_prefix())
    }

    /// Group the `identifier` children of `node` into dotted paths,
    /// splitting where the source bytes between two identifiers contain
    /// no `.` (e.g. `open Nat List` -> ["Nat", "List"], while
    /// `import A.B.C` -> ["A.B.C"]).
    fn dotted_groups(&self, node: Node) -> Vec<String> {
        let mut groups = Vec::new();
        let mut cur = String::new();
        let mut prev_end: Option<usize> = None;
        for child in child_nodes(node) {
            if !is_ident(child.kind()) {
                continue;
            }
            let txt = self.text(child);
            match prev_end {
                Some(pe) => {
                    let gap = self.source.get(pe..child.start_byte()).unwrap_or(&[]);
                    if gap.contains(&b'.') {
                        cur.push('.');
                        cur.push_str(txt);
                    } else {
                        if !cur.is_empty() {
                            groups.push(std::mem::take(&mut cur));
                        }
                        cur = txt.to_string();
                    }
                }
                None => cur = txt.to_string(),
            }
            prev_end = Some(child.end_byte());
        }
        if !cur.is_empty() {
            groups.push(cur);
        }
        groups
    }

    fn first_ident<'t>(&self, node: Node<'t>) -> Option<Node<'t>> {
        child_nodes(node).into_iter().find(|c| is_ident(c.kind()))
    }

    // --- top-level, namespace-aware walk ---------------------------------

    fn visit(&mut self, node: Node) {
        match node.kind() {
            "decorated_declaration" => self.visit_decorated(node),
            "import" => self.extract_imports(node, "import"),
            "open" => self.extract_imports(node, "open"),
            "namespace" => self.ns_stack.push(self.dotted_groups(node).join(".")),
            "section" => self.ns_stack.push(String::new()),
            "end" => {
                self.ns_stack.pop();
            }
            // `constant`/`opaque`/`axiom` are separate grammar node kinds
            // from `definition` (unlike `instance`, which the grammar
            // aliases to `definition`) but shape identically enough — a
            // leading name identifier, optional body — that
            // `extract_definition` handles all four.
            "definition" | "constant" | "opaque" | "axiom" => self.extract_definition(node),
            "structure" => self.extract_structure(node),
            "inductive" | "class_inductive" => self.extract_inductive(node),
            "example" => self.record(node, false),
            _ => {
                for child in child_nodes(node) {
                    self.visit(child);
                }
            }
        }
    }

    /// A declaration plus its `@[..]` attribute preamble. Collect the
    /// attribute names into `pending_attrs`, visit the wrapped declaration
    /// (which reads them via `push_def`), then clear.
    fn visit_decorated(&mut self, node: Node) {
        let mut attrs = Vec::new();
        for child in child_nodes(node) {
            if child.kind() == "attributes" {
                for entry in child_nodes(child) {
                    if entry.kind() == "attribute_entry" {
                        // `attribute_entry` is `name arg*`; the name is its
                        // first dotted group (drop priority/string args).
                        if let Some(name) = self.dotted_groups(entry).into_iter().next() {
                            attrs.push(format!("@[{name}]"));
                        }
                    }
                }
            }
        }
        self.pending_attrs = attrs;
        for child in child_nodes(node) {
            self.visit(child);
        }
        self.pending_attrs.clear();
    }

    fn extract_imports(&mut self, node: Node, kind: &str) {
        let line = (node.start_position().row as u32) + 1;
        let byte = node.start_byte() as u32;
        for path in self.dotted_groups(node) {
            if path.is_empty() {
                continue;
            }
            self.facts.imports.push(ImportRecord {
                kind: kind.to_string(),
                path,
                alias: String::new(),
                site_line: line,
                site_byte: byte,
            });
        }
    }

    fn push_def(&mut self, name: &str, qualified: String, variant: DefVariant, node: Node, sig: String) {
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(),
            qualified_name: qualified,
            variant,
            start_line: (node.start_position().row as u32) + 1,
            end_line: (node.end_position().row as u32) + 1,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: sig,
            visibility: self.visibility(node),
            attributes: self.pending_attrs.clone(),
            ..Default::default()
        });
    }

    fn visibility(&self, node: Node) -> String {
        let t = self.text(node);
        // Only the leading modifier keywords matter; cap the scan to ~48 bytes.
        // Lean source is full of multi-byte Unicode (ℝ, ℕ, ₀); floor_char_boundary
        // both clamps past-the-end and steps back to a char boundary.
        let head = &t[..t.floor_char_boundary(48)];
        if head.contains("private") {
            "private".to_string()
        } else if head.contains("protected") {
            "protected".to_string()
        } else {
            String::new()
        }
    }

    fn extract_definition(&mut self, node: Node) {
        // `def`/`theorem`/`lemma`/`abbrev`/`instance`/`axiom`/`opaque`/
        // `constant`. The grammar's `_name` production is inlined
        // (`Cert.sosDeg10` parses as two direct `identifier` children
        // joined by a `.` token, not one node), so the name is the first
        // *dotted group* of `node`'s direct identifier children, not just
        // its first identifier — otherwise `axiom Cert.sosDeg10 : _` would
        // be recorded as a bare `Cert`. Anonymous instances have no name.
        //
        // `instance` is aliased to `definition` in the grammar; its first
        // token is the `instance` keyword. Mark such defs so consumers know
        // they are reached by typeclass resolution rather than by name — a
        // zero-in-degree instance is implicitly used, not dead.
        let is_instance = child_nodes(node)
            .first()
            .is_some_and(|c| c.kind() == "instance");
        if is_instance {
            self.pending_attrs.push("instance".to_string());
        }
        if let Some(dotted) = self.dotted_groups(node).into_iter().next() {
            // `simple_name` must be the last path segment (`sosDeg10`, not
            // `Cert.sosDeg10`) to match how call sites are recorded: a
            // dotted reference like `Cert.sosDeg10` splits into
            // `receiver_hint: "Cert", name: "sosDeg10"` (see the
            // `projection` arm of `record`), and intra-file linking matches
            // candidates by `simple_name == ref.name` before narrowing by
            // receiver/owner. The full dotted form still lives in the
            // qualified name below, so `owner_from_qn` recovers `Cert` as
            // the owner on the resolver side.
            let simple = dotted.rsplit('.').next().unwrap_or(&dotted).to_string();
            let qn = self.qn(&dotted);
            let sig = lean_signature(self.text(node));
            self.push_def(&simple, qn, DefVariant::FreeFunction, node, sig);
        }
        if is_instance {
            self.pending_attrs.pop();
        }
        // Body call sites (bare identifiers — the name, binders, type
        // annotations — are not recorded; only application/projection/
        // tactic references are).
        self.record(node, false);
    }

    fn extract_structure(&mut self, node: Node) {
        let Some(name_node) = self.first_ident(node) else {
            return;
        };
        let type_name = self.text(name_node).to_string();
        let qn = self.qn(&type_name);
        let sig = lean_signature(self.text(node));
        self.push_def(&type_name, qn, DefVariant::FreeFunction, node, sig);

        for child in child_nodes(node) {
            if child.kind() == "structure_field" {
                if let Some(fnode) = self.first_ident(child) {
                    let field = self.text(fnode).to_string();
                    let q = self.qn(&format!("{type_name}.{field}"));
                    self.push_def(&field, q, DefVariant::Property, fnode, String::new());
                }
            }
        }
        self.record(node, false);
    }

    fn extract_inductive(&mut self, node: Node) {
        let Some(name_node) = self.first_ident(node) else {
            return;
        };
        let type_name = self.text(name_node).to_string();
        let qn = self.qn(&type_name);
        let sig = lean_signature(self.text(node));
        self.push_def(&type_name, qn, DefVariant::FreeFunction, node, sig);

        for child in child_nodes(node) {
            if child.kind() == "constructor" {
                if let Some(cnode) = self.first_ident(child) {
                    let ctor = self.text(cnode).to_string();
                    let q = self.qn(&format!("{type_name}.{ctor}"));
                    self.push_def(&ctor, q, DefVariant::Constructor, cnode, String::new());
                }
            }
        }
        self.record(node, false);
    }

    // --- call-site recording ---------------------------------------------

    fn push_ref(&mut self, name: &str, receiver: &str, node: Node) {
        if name.is_empty() || name == "_" {
            return;
        }
        self.facts.references.push(RefRecord {
            name: name.to_string(),
            receiver_hint: receiver.to_string(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }

    /// Record references reachable from `node`. `bare_ok` enables
    /// recording standalone identifiers — true only inside tactic
    /// arguments, where bare identifiers name lemmas/defs being applied.
    fn record(&mut self, node: Node, bare_ok: bool) {
        match node.kind() {
            k if is_ident(k) => {
                if bare_ok {
                    let name = self.text(node).to_string();
                    if !is_tactic_modifier(&name) {
                        self.push_ref(&name, "", node);
                    }
                }
            }
            "tactic_config" => {
                // `[lemma, ←lemma, ...]` — the bracketed lemma list shared by
                // `simp`/`rw`/`rewrite`. Every operand is a lemma/def
                // reference, even when this node is reached from a non-tactic
                // context (we do not special-case the `tactic_rewrite` node;
                // the generic walk descends into its config here), so force
                // bare recording regardless of the incoming flag.
                for c in child_nodes(node) {
                    self.record(c, true);
                }
            }
            "application" => {
                let children = child_nodes(node);
                if let Some(first) = children.first() {
                    if is_ident(first.kind()) {
                        let name = self.text(*first).to_string();
                        if !(bare_ok && is_tactic_modifier(&name)) {
                            self.push_ref(&name, "", *first);
                        }
                    } else {
                        self.record(*first, bare_ok);
                    }
                }
                // Propagate `bare_ok`: in tactic mode (`simp only [..]`) the
                // operands are lemmas in a nested `list`, so they must keep
                // being recorded; in term mode this stays `false`, leaving
                // ordinary call arguments (locals/binders) untouched.
                for c in children.iter().skip(1) {
                    self.record(*c, bare_ok);
                }
            }
            "projection" => {
                let children = child_nodes(node);
                if let (Some(first), Some(last)) = (children.first(), children.last()) {
                    if is_ident(last.kind()) {
                        let recv = collapse(self.text(*first));
                        let name = self.text(*last).to_string();
                        self.push_ref(&name, &recv, *last);
                    }
                    for c in children.iter().take(children.len().saturating_sub(1)) {
                        self.record(*c, false);
                    }
                }
            }
            "tactic_apply" => {
                // First identifier is the tactic name (simp/exact/apply);
                // the remaining operands are the lemma/def references.
                let mut tactic_named = false;
                for c in child_nodes(node) {
                    if !tactic_named && is_ident(c.kind()) {
                        tactic_named = true;
                        continue;
                    }
                    self.record(c, true);
                }
            }
            _ => {
                for c in child_nodes(node) {
                    self.record(c, bare_ok);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_lean4::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        LeanPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.lean"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn plugin_loads() {
        let p = LeanPlugin;
        assert_eq!(p.id(), "lean");
        assert!(p.extensions().contains(&".lean"));
    }

    #[test]
    fn definitions_are_namespaced() {
        let facts = extract(
            r#"
namespace Foo.Bar

def double (n : Nat) : Nat := n + n

theorem double_eq (n : Nat) : double n = 2 * n := by
  simp [double]

end Foo.Bar

def topLevel : Nat := 0
"#,
        );
        let qns: Vec<&str> = facts
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Foo.Bar.double"), "got {qns:?}");
        assert!(qns.contains(&"Foo.Bar.double_eq"), "got {qns:?}");
        // After `end`, the namespace prefix is dropped.
        assert!(qns.contains(&"topLevel"), "got {qns:?}");
    }

    #[test]
    fn axiom_opaque_and_constant_are_definitions() {
        // Mirrors the shape that went unreported: a certificate axiom named
        // and referenced via a dotted projection (`Cert.sosDeg10`), plus
        // `opaque`/`constant` — three grammar node kinds distinct from
        // `definition` that the walker previously fell through on, so the
        // axiom never became a callable at all.
        let facts = extract(
            r#"
axiom Cert.sosDeg10 : (1:Nat) = 1

opaque hidden : Nat := 0

constant legacy : Nat

theorem restsOnAxiom : (1:Nat) = 1 := Cert.sosDeg10
"#,
        );
        let qns: Vec<&str> = facts
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Cert.sosDeg10"), "got {qns:?}");
        assert!(qns.contains(&"hidden"), "got {qns:?}");
        assert!(qns.contains(&"legacy"), "got {qns:?}");
        // `simple_name` is the last path segment, matching how the
        // `Cert.sosDeg10` reference below splits into receiver "Cert" +
        // name "sosDeg10" — required for intra-file linking to connect the
        // two (it candidate-matches on `simple_name == ref.name`).
        let sn = facts
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Cert.sosDeg10")
            .map(|d| d.simple_name.as_str());
        assert_eq!(sn, Some("sosDeg10"));
        let refs: Vec<&str> = facts.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"sosDeg10"), "got {refs:?}");
    }

    #[test]
    fn imports_and_opens() {
        let facts = extract(
            r#"
import Mathlib.Data.List.Basic
open Nat List
"#,
        );
        let imports: Vec<(&str, &str)> = facts
            .imports
            .iter()
            .map(|i| (i.kind.as_str(), i.path.as_str()))
            .collect();
        assert!(imports.contains(&("import", "Mathlib.Data.List.Basic")), "got {imports:?}");
        // `open Nat List` is two separate opened namespaces, not "Nat.List".
        assert!(imports.contains(&("open", "Nat")), "got {imports:?}");
        assert!(imports.contains(&("open", "List")), "got {imports:?}");
    }

    #[test]
    fn application_and_projection_calls() {
        let facts = extract(
            r#"
def usePoint (p : Point) : Nat :=
  double (p.x) + helper p
"#,
        );
        let names: Vec<&str> = facts.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"double"), "got {names:?}");
        assert!(names.contains(&"helper"), "got {names:?}");
        // `p.x` projection is recorded by field name with a receiver hint.
        assert!(
            facts
                .references
                .iter()
                .any(|r| r.name == "x" && r.receiver_hint == "p"),
            "projection not recorded: {:?}",
            facts.references
        );
    }

    #[test]
    fn tactic_references_are_captured() {
        let facts = extract(
            r#"
theorem t : True := by
  exact myLemma
"#,
        );
        let names: Vec<&str> = facts.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"myLemma"), "got {names:?}");
        // The tactic keyword itself is not a reference.
        assert!(!names.contains(&"exact"), "tactic name leaked: {names:?}");
    }

    #[test]
    fn rewrite_lemmas_are_captured() {
        // Regression: `rw`/`rewrite` parse as `tactic_rewrite`, whose config
        // list was walked in non-bare mode, so bare lemma names were dropped
        // (only dotted ones survived, via the projection branch). Such lemmas
        // showed up as false zero-in-degree orphans.
        let facts = extract(
            r#"
theorem t (n : Nat) : n + 0 = n := by
  rw [add_zero_eq, ← mul_one_eq]
  rewrite [Demo.dotted_eq] at h
"#,
        );
        let names: Vec<&str> = facts.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"add_zero_eq"), "bare rw lemma missed: {names:?}");
        assert!(names.contains(&"mul_one_eq"), "←-prefixed rw lemma missed: {names:?}");
        assert!(names.contains(&"dotted_eq"), "dotted rewrite lemma missed: {names:?}");
        // The `at h` target is a local hypothesis, not a lemma.
        assert!(!names.contains(&"h"), "rewrite `at` target leaked: {names:?}");
    }

    #[test]
    fn simp_only_lemmas_are_captured() {
        // Regression: `simp only [..]` parses as `tactic_apply(simp,
        // application(only, list[..]))`. The application branch reset its
        // arguments to non-bare mode, dropping the lemmas in the nested list,
        // and recorded the `only` modifier as a bogus reference.
        let facts = extract(
            r#"
theorem t (n : Nat) : n * 1 = n := by
  simp only [foo_eq, Set.mem_def]
"#,
        );
        let names: Vec<&str> = facts.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"foo_eq"), "simp-only lemma missed: {names:?}");
        assert!(names.contains(&"mem_def"), "dotted simp-only lemma missed: {names:?}");
        // `only` is a tactic modifier, not a lemma reference.
        assert!(!names.contains(&"only"), "`only` modifier leaked: {names:?}");
    }

    #[test]
    fn attributes_and_instance_marker_are_captured() {
        // Implicitly-used decls (a `@[simp]` lemma fired by bare `simp`, an
        // `instance` chosen by typeclass resolution) carry no by-name call
        // edge, so they look like orphans. Capturing their attributes lets a
        // dead-code pass exclude them.
        let facts = extract(
            r#"
@[simp] theorem foo_eq (n : Nat) : n + 0 = n := by rfl
@[simp, reducible] def bar : Nat := 0
instance instAddFoo : Add Foo where add := myAdd
def plain : Nat := 1
"#,
        );
        let attrs = |name: &str| -> Vec<String> {
            facts
                .definitions
                .iter()
                .find(|d| d.simple_name == name)
                .map(|d| d.attributes.clone())
                .unwrap_or_default()
        };
        assert!(attrs("foo_eq").contains(&"@[simp]".to_string()), "{:?}", attrs("foo_eq"));
        assert!(attrs("bar").contains(&"@[simp]".to_string()), "{:?}", attrs("bar"));
        assert!(attrs("bar").contains(&"@[reducible]".to_string()), "{:?}", attrs("bar"));
        assert!(attrs("instAddFoo").contains(&"instance".to_string()), "{:?}", attrs("instAddFoo"));
        // A plain decl carries no attributes (and the marker does not leak).
        assert!(attrs("plain").is_empty(), "attrs leaked onto plain decl: {:?}", attrs("plain"));
    }

    #[test]
    fn structures_and_inductives() {
        let facts = extract(
            r#"
structure Point where
  x : Nat
  y : Nat

inductive Color where
  | red
  | green
"#,
        );
        let qns: Vec<&str> = facts
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Point"), "got {qns:?}");
        assert!(qns.contains(&"Point.x"), "got {qns:?}");
        assert!(qns.contains(&"Color"), "got {qns:?}");
        assert!(qns.contains(&"Color.red"), "got {qns:?}");
    }

    #[test]
    fn unicode_signature_does_not_panic() {
        // Regression: `visibility` capped its scan at byte 48 without honoring
        // char boundaries, so a multi-byte glyph (ℝ, ℕ, ₀) straddling that
        // offset panicked. Sweep padding lengths so the glyph lands across the
        // cap at some offset, and assert extraction succeeds for all of them.
        for pad in 30..60 {
            let name = "f".repeat(pad);
            let src = format!("def {name} : ℝ → ℕ := fun x₀ => x₀\n");
            let facts = extract(&src);
            assert!(
                facts.definitions.iter().any(|d| d.simple_name == name),
                "definition not extracted for pad={pad}"
            );
        }
    }
}
