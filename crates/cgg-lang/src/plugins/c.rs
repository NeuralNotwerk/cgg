//! C plugin — preprocessor-aware callable extraction.
//!
//! * `function_definition` → callable (DefVariant::FreeFunction).
//! * `declaration` with a `function_declarator` child → prototype
//!   (DefVariant::FreeFunction, zero-length body).
//! * `preproc_include` with a quoted path → ImportRecord kind="include".
//! * `call_expression` → RefRecord.
//! * Macro invocations that look like calls but have no matching def
//!   are resolved downstream as `reason: "macro-call-site"`.

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct CPlugin;

impl LanguagePlugin for CPlugin {
    fn id(&self) -> &'static str {
        "c"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".c", ".h"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            unreachable: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "c");
        let mut w = CWalker {
            source,
            facts: &mut facts,
            ctx: *ctx,
            skip_call: None,
        };
        w.walk(tree.root_node());
        let mut out = facts;
        if ctx.reference_sites {
            let (refs, members) = super::c_values::collect(ctx, tree, source);
            out.references.extend(refs);
            out.data_members = members;
        }
        out.macro_invocations =
            super::c_macro::scan_invocations(tree.root_node(), source, false);
        if ctx.deadcode_signals {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::C_LIKE);
        }
        out
    }
}

/// The definitions and calls of a macro expansion (see
/// [`super::c_macro`]), extracted as a file's would be.
pub(super) fn extract_fragment(
    ctx: &crate::ExtractCtx<'_>,
    file: FileId,
    path: &Path,
    tree: &Tree,
    source: &[u8],
) -> FileFacts {
    let mut facts = FileFacts::new(file, path.to_path_buf(), "c");
    let mut w = CWalker {
        source,
        facts: &mut facts,
        ctx: *ctx,
        skip_call: None,
    };
    w.walk(tree.root_node());
    facts
}

struct CWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    /// The registrar-verb gate. C hands control to a function by passing
    /// its name to a runtime API (`signal(SIGINT, on_sigint)`); without
    /// the capture those handlers have no reference at all.
    ctx: crate::ExtractCtx<'a>,
    /// The swallowed function header inside the definition being walked
    /// (see [`swallowed_header`]): not a call.
    skip_call: Option<usize>,
}

impl<'a> CWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => {
                self.record_function_def(node);
                // When a macro invocation with no `;` precedes a function,
                // the declarator holds the swallowed header (`helper(void)`),
                // which is the definition's name, not a call. Everything
                // else is walked as before (an attribute macro after the
                // parameters stays a reference to that macro).
                let swallowed = node
                    .child_by_field_name("declarator")
                    .and_then(swallowed_header)
                    .map(|n| n.id());
                let prev = std::mem::replace(&mut self.skip_call, swallowed);
                self.walk_children(node);
                self.skip_call = prev;
                return;
            }
            "declaration" => {
                // Prototype: `int foo(int a, int b);`
                self.try_record_prototype(node);
                self.walk_children(node);
                return;
            }
            "preproc_function_def" => {
                // #define FOO(x) ... — record as a callable so calls
                // to FOO resolve intra-file rather than being tagged
                // unresolved.
                self.record_macro_def(node);
                return;
            }
            "preproc_include" => {
                self.record_include(node);
                return;
            }
            "call_expression" if self.skip_call == Some(node.id()) => {
                self.walk_children(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                if let Some(callee) = node.child_by_field_name("function") {
                    let context = self.text(callee).to_string();
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

    fn record_function_def(&mut self, node: Node) {
        let Some(decl) = node.child_by_field_name("declarator") else {
            return;
        };
        // `FIELDS(DECL)\nint helper(void) { … }`: a macro invocation with
        // no trailing `;` makes the grammar read the invocation as the
        // declarator (`FIELDS(DECL)`) and the real header as stray
        // children of it — a `call_expression` `helper(void)`. That
        // header names the function.
        let swallowed = swallowed_header(decl).filter(|call| {
            call.child_by_field_name("function")
                .is_some_and(|f| f.kind() == "identifier")
        });
        let name = swallowed
            .and_then(|call| call.child_by_field_name("function"))
            .map(|f| self.text(f).to_string())
            .unwrap_or_else(|| self.fn_name_from_declarator(decl));
        if name.is_empty() {
            return;
        }
        // The definition starts at its own header's return type, not at
        // the invocations the grammar folded in front of it: those are
        // items of their own (see `c_macro`).
        // `LIST(DECL)\nint g(void) {…}` parses the invocation as the return
        // type (`macro_type_specifier`) and the real one as an error.
        let after_macro_type = node
            .child_by_field_name("type")
            .filter(|t| t.kind() == "macro_type_specifier")
            .and_then(|t| t.next_named_sibling())
            .filter(|n| n.is_error());
        let start = swallowed
            .and_then(|call| call.prev_named_sibling())
            .or(after_macro_type)
            .unwrap_or(node);
        let (_, el) = line_range(node);
        let sl = start.start_position().row as u32 + 1;
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(),
            qualified_name: name,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: start.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
            // `function_definition` is the body. Prototypes stay at the
            // default `has_body: false`, which is what
            // `unify_declarations` tells a declaration from a definition.
            has_body: true,
            ..Default::default()
        });
    }

    fn try_record_prototype(&mut self, node: Node) {
        // A declaration that contains a function_declarator is a prototype.
        let mut c = node.walk();
        for child in node.children(&mut c) {
            // `char *f(void);` wraps the function declarator in a pointer
            // declarator for its return type.
            if matches!(child.kind(), "function_declarator" | "pointer_declarator")
                && !self.fn_name_from_declarator(child).is_empty()
            {
                let name = self.fn_name_from_declarator(child);
                if !name.is_empty() {
                    let (sl, el) = line_range(node);
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
                return;
            }
        }
    }

    /// See [`fn_name_from_declarator`].
    fn fn_name_from_declarator(&self, decl: Node) -> String {
        fn_name_from_declarator(decl, self.source)
    }

    fn record_macro_def(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }
        let (sl, el) = line_range(node);
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
            attributes: vec!["macro".to_string()],
            ..Default::default()
        });
        if let Some(m) = super::c_macro::record_fn_macro(node, self.source) {
            self.facts.fn_macros.push(m);
        }
    }

    fn record_include(&mut self, node: Node) {
        let Some(path_node) = node.child_by_field_name("path") else {
            return;
        };
        let kind = path_node.kind();
        // Only project-local (quoted) includes become import records.
        if kind == "string_literal" || kind == "string_content" {
            let raw = self.text(path_node);
            let path = raw.trim_matches('"').to_string();
            if !path.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "include".into(),
                    path,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
        // A system include (`<signal.h>`, `<gtest/gtest.h>`) names no
        // file in the tree, so it stays out of the `include` kind the
        // cross-file resolver follows. It is still the only evidence that
        // a library is in use, though, and framework detection needs it —
        // without this no C/C++ rule keyed on a system header could ever
        // fire. Recorded under a distinct kind so the two consumers do not
        // interfere.
        else if kind == "system_lib_string" {
            let path = self.text(path_node).trim_matches(['<', '>']).to_string();
            if !path.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "system-include".into(),
                    path,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        if let Some(r) = call_ref(node, self.source) {
            self.facts.references.push(r);
        }
    }
}

/// The function a declarator declares, looking through the pointer,
/// parenthesis and attribute layers a return type wraps around its
/// `function_declarator` (`char *f(void)`, `int (*f(void))(int)`).
/// Empty when the identifier is not the direct declarator of a
/// function declarator: `int (*fp)(int);` declares a pointer variable,
/// not a function. Shared with the Objective-C plugin, whose functions
/// are C's.
pub(crate) fn fn_name_from_declarator(decl: Node, src: &[u8]) -> String {
    fn inner(n: Node) -> Option<Node> {
        n.child_by_field_name("declarator")
            .or_else(|| n.named_children(&mut n.walk()).last())
    }
    let text = |n: Node| n.utf8_text(src).unwrap_or("").to_string();
    match decl.kind() {
        "function_declarator" => match decl.child_by_field_name("declarator") {
            Some(d) if d.kind() == "identifier" => text(d),
            Some(d) => fn_name_from_declarator(d, src),
            None => String::new(),
        },
        // `int (f)(int)` declares the function `f` (a macro-built name
        // `RET NAME_MACRO(f)(…)` parses so); `int (*fp)(int)` does not.
        "parenthesized_declarator" => match inner(decl) {
            Some(d) if d.kind() == "identifier" => text(d),
            Some(d) => fn_name_from_declarator(d, src),
            None => String::new(),
        },
        "pointer_declarator" | "attributed_declarator" => {
            inner(decl).map_or_else(String::new, |d| fn_name_from_declarator(d, src))
        }
        _ => String::new(),
    }
}

/// The call a C `call_expression` makes: `f(…)` by name, `s.f(…)` /
/// `p->f(…)` through a field. Shared with the Objective-C plugin, whose
/// plain function calls are C's.
pub(crate) fn call_ref(node: Node, src: &[u8]) -> Option<RefRecord> {
    let func = node.child_by_field_name("function")?;
    let text = |n: Node| n.utf8_text(src).unwrap_or("").to_string();
    let (name, recv) = match func.kind() {
        "identifier" => (text(func), String::new()),
        "field_expression" => {
            let arg = func
                .child_by_field_name("argument")
                .map(text)
                .unwrap_or_default();
            let field = func
                .child_by_field_name("field")
                .map(text)
                .unwrap_or_default();
            (field, arg)
        }
        _ => return None,
    };
    if name.is_empty() {
        return None;
    }
    Some(RefRecord {
        name,
        receiver_hint: recv,
        site_line: (node.start_position().row as u32) + 1,
        site_byte: node.start_byte() as u32,
        ..Default::default()
    })
}

fn line_range(n: Node) -> (u32, u32) {
    (
        (n.start_position().row as u32) + 1,
        (n.end_position().row as u32) + 1,
    )
}

/// The function header a macro invocation without `;` pushed inside a
/// function declarator: a `call_expression` among the declarator's own
/// children that follows a return-type token (`int helper(void)` is
/// `identifier` `int`, then the call). A call right after the parameter
/// list is an attribute macro (`f(int a) ATTR_NONNULL(1)`), not a header.
fn swallowed_header(decl: Node) -> Option<Node> {
    if decl.kind() != "function_declarator" {
        return None;
    }
    // The outer "parameters" are the macro's arguments (`FIELDS(DECL)`):
    // bare names with no declarator. A real parameter list names its
    // parameters (or is `void` / empty), and then a call after it is an
    // attribute macro (`f(win_T *w) ATTR_PURE ATTR_NONNULL(1)`).
    let params = decl.child_by_field_name("parameters")?;
    let mut pc = params.walk();
    let ps: Vec<Node> = params.named_children(&mut pc).collect();
    let macro_args = !ps.is_empty()
        && ps.iter().all(|p| {
            p.kind() == "parameter_declaration"
                && p.child_by_field_name("declarator").is_none()
                && p.child_by_field_name("type")
                    .is_some_and(|t| t.kind() == "type_identifier")
        });
    if !macro_args {
        return None;
    }
    let mut c = decl.walk();
    let kids: Vec<Node> = decl.named_children(&mut c).collect();
    kids.windows(2)
        .rev()
        .find(|w| {
            w[1].kind() == "call_expression"
                && matches!(
                    w[0].kind(),
                    "identifier"
                        | "primitive_type"
                        | "type_identifier"
                        | "sized_type_specifier"
                        | "storage_class_specifier"
                        | "type_qualifier"
                )
        })
        .map(|w| w[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        CPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.c"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn pointer_returning_functions_are_named_and_pointer_variables_are_not() {
        let f = extract(
            "char *make(void) { return 0; }\nstatic const char **two(int n) { return 0; }\nint (*get(void))(int) { return 0; }\nchar *proto(void);\nint (*fp)(int);\n",
        );
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        let g = extract(
            "static ETHR_INLINE ethr_sint_t ETHR_ATMC_FUNC__(xchg)(ethr_atomic_t *var, ethr_sint_t val)\n{\n  return 0;\n}\n",
        );
        assert!(g.definitions.iter().any(|d| d.simple_name == "xchg"));
        for n in ["make", "two", "get", "proto"] {
            assert!(names.contains(&n), "{n} missing: {names:?}");
        }
        // A pointer-to-function variable declares no function.
        assert!(!names.contains(&"fp"), "{names:?}");
    }

    #[test]
    fn a_macro_invocation_without_semicolon_does_not_swallow_the_next_function() {
        // `LIST(X)` with no `;` makes the grammar read it as the
        // declarator and the real header as stray children of it.
        let f = extract("LIST(DECL)\nint helper(void) { return other(); }\n");
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert_eq!(names, vec!["helper"], "{names:?}");
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        // The swallowed header is not a call; the body's call is.
        assert!(!refs.contains(&"helper"), "{refs:?}");
        assert!(refs.contains(&"other"), "{refs:?}");
        // Near-miss: an attribute macro after the parameter list names
        // nothing; the function keeps its own name.
        let f = extract(
            "bool f(int *p)\n  ATTR_PURE ATTR_NONNULL(1)\n{\n  return g(p);\n}\n",
        );
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert_eq!(names, vec!["f"], "{names:?}");
    }

    #[test]
    fn function_definitions_extracted() {
        let src = "int add(int a, int b) { return a+b; }\nvoid greet() {}\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"add"), "got: {names:?}");
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(
            f.definitions
                .iter()
                .all(|d| d.variant == DefVariant::FreeFunction && d.has_body)
        );
    }

    #[test]
    fn prototype_recorded() {
        let src = "int add(int a, int b);\nint add(int a, int b) { return a+b; }\n";
        let f = extract(src);
        // Both prototype and definition are recorded (dedup is Task 9).
        let count = f
            .definitions
            .iter()
            .filter(|d| d.simple_name == "add")
            .count();
        assert_eq!(count, 2);
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "add" && !d.has_body),
            "prototype must stay has_body false"
        );
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "add" && d.has_body),
            "function_definition must set has_body"
        );
    }

    #[test]
    fn include_directive_captured() {
        let src = "#include \"helpers.h\"\n#include <stdio.h>\nvoid f() {}\n";
        let f = extract(src);
        // Both forms are recorded, under different kinds. Only the quoted
        // one names a file in the tree, so only it is an `include` the
        // cross-file resolver follows; the system one is the sole
        // evidence a library is in use and framework detection needs it.
        assert_eq!(f.imports.len(), 2);
        let quoted = f.imports.iter().find(|i| i.kind == "include").unwrap();
        assert_eq!(quoted.path, "helpers.h");
        let sys = f
            .imports
            .iter()
            .find(|i| i.kind == "system-include")
            .expect("system include recorded");
        assert_eq!(sys.path, "stdio.h");
    }

    #[test]
    fn call_expressions_captured() {
        let src = "void f() { add(1,2); ptr->run(); }\n";
        let f = extract(src);
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("add", "")), "got: {refs:?}");
        // ptr->run() is a field_expression
        assert!(refs.contains(&("run", "ptr")), "got: {refs:?}");
    }

    // B4: a function handed to a runtime registration API by name is a
    // value reference the framework rules can root. An identifier passed
    // to an ordinary call is not captured by this path.
    #[test]
    fn functions_passed_to_runtime_registrars_are_value_refs() {
        let src = "void h(int s) {}\nvoid g(void) {}\n\
                   int main(void) { signal(SIGINT, h); atexit(g); log_it(g); return 0; }\n";
        let f = extract(src);
        let vals: Vec<(&str, &str)> = f
            .references
            .iter()
            .filter(|r| r.receiver_hint == cgg_core::VALUE_REF_HINT)
            .map(|r| (r.name.as_str(), r.context.as_str()))
            .collect();
        assert!(vals.contains(&("h", "signal")), "got: {vals:?}");
        assert!(vals.contains(&("g", "atexit")), "got: {vals:?}");
        assert!(!vals.iter().any(|(_, c)| *c == "log_it"), "got: {vals:?}");
    }

    #[test]
    fn macro_call_looks_like_call() {
        let src = "#define SQUARE(x) ((x)*(x))\nint f() { return SQUARE(3); }\n";
        let f = extract(src);
        // SQUARE(3) is parsed as a call_expression by tree-sitter-c.
        assert!(
            f.references.iter().any(|r| r.name == "SQUARE"),
            "macro call not captured"
        );
    }
}
