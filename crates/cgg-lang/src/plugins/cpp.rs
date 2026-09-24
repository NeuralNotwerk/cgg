//! C++ plugin — namespace/class-aware callable extraction.
//!
//! Extends the C model with:
//! * `namespace_definition` scope stack.
//! * `class_specifier` / `struct_specifier` type scope.
//! * Methods inside class bodies (including constructors/destructors).
//! * `qualified_identifier` in call expressions (`ns::fn()`).
//! * `field_expression` for `obj.method()` / `ptr->method()`.
//!
//! An out-of-line definition (`void Robot::on_gcode_received(...) { }`)
//! keeps the same simple name as the header declaration (`on_gcode_received`),
//! and [`unify_declarations`] drops that declaration when the body is in
//! the tree. Otherwise every call binds to the empty prototype and the
//! body — which holds the outgoing edges — has no callers.

use std::path::Path;

use cgg_core::{
    DefRecord, DefVariant, FieldType, FileFacts, ImportRecord, MacroAlias, MemberPtrTake,
    RefRecord, first_template_arg, ids::FileId,
};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct CppPlugin;

impl LanguagePlugin for CppPlugin {
    fn id(&self) -> &'static str {
        "cpp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cc", ".cpp", ".cxx", ".C", ".hpp", ".hh", ".hxx"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            attributes: true,
            unreachable: true,
            impls: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_cpp::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "cpp");
        let mut w = CppWalker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            class_stack: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        let mut out = facts;
        if ctx.deadcode_signals {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::C_LIKE);
        }
        out
    }
}

struct CppWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Class and struct names currently open. Namespaces stay on `scope`
    /// only, so a field's owner is the class and not the namespace.
    class_stack: Vec<String>,
    /// Base types of the enclosing class, innermost last. A method
    /// carries its owner's supertypes because the resolver fans a
    /// virtual call out to overrides by walking this list.
    bases: Vec<Vec<String>>,
    /// Needed for the registrar-verb gate. Without it C++ captured no
    /// argument-position handler at all, so `run_handler(my_handler)` —
    /// the one entry point an aws-lambda-cpp binary has — referenced
    /// nothing and read as dead.
    ctx: crate::ExtractCtx<'a>,
}

impl<'a> CppWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}::{simple}", self.scope.join("::"))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "namespace_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    // Each translation unit's anonymous namespace is a
                    // distinct scope. Leaving it unnamed made `helper` in
                    // two `.cpp` files the same qualified name, so a body
                    // there would absorb an unrelated header prototype.
                    self.scope.push("(anonymous)".to_string());
                    self.walk_children(node);
                    self.scope.pop();
                }
                return;
            }
            "class_specifier" | "struct_specifier" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let class_bases = super::attrs::base_types(node, self.source);
                if !name.is_empty() {
                    self.scope.push(name.clone());
                    self.class_stack.push(name);
                    self.bases.push(class_bases);
                    self.walk_children(node);
                    self.bases.pop();
                    self.class_stack.pop();
                    self.scope.pop();
                } else {
                    self.bases.push(class_bases);
                    self.walk_children(node);
                    self.bases.pop();
                }
                return;
            }
            "function_definition" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "template_declaration" => {
                // template<typename T> void foo() {} — unwrap to find the function inside
                self.walk_children(node);
                return;
            }
            // Method declarations inside a class are `field_declaration`,
            // not `declaration`. Missing that node is why a header
            // prototype never existed to absorb into the out-of-line body.
            "declaration" | "field_declaration" => {
                if node.kind() == "field_declaration" {
                    self.record_field(node);
                } else {
                    self.record_local(node);
                }
                self.try_record_prototype(node);
                self.walk_children(node);
                return;
            }
            "preproc_function_def" => {
                self.record_function_macro(node);
                return;
            }
            "preproc_def" => {
                self.record_object_macro(node);
                return;
            }
            "alias_declaration" => {
                // `using Rect = TRect<float>;`
                if let (Some(n), Some(t)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("type"),
                ) {
                    self.record_type_alias(
                        self.text(n).to_string(),
                        self.text(t).to_string(),
                    );
                }
                self.walk_children(node);
                return;
            }
            "type_definition" => {
                // `typedef TRect<float> Rect;` — only a plain name
                // declarator; pointer and function-pointer typedefs name
                // no class.
                if let (Some(t), Some(d)) = (
                    node.child_by_field_name("type"),
                    node.child_by_field_name("declarator"),
                ) && d.kind() == "type_identifier"
                {
                    self.record_type_alias(
                        self.text(d).to_string(),
                        self.text(t).to_string(),
                    );
                }
                self.walk_children(node);
                return;
            }
            "preproc_include" => {
                self.record_include(node);
                return;
            }
            "new_expression" => {
                self.record_new(node);
                self.walk_children(node);
                return;
            }
            "pointer_expression" => {
                self.record_member_ptr_take(node);
                self.walk_children(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                // Shape B: `run_handler(my_handler)`.
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

    fn record_function(&mut self, node: Node) {
        let Some(decl) = node.child_by_field_name("declarator") else {
            return;
        };
        let Some(fn_decl) = unwrap_function_declarator(decl) else {
            return;
        };
        let info = self.fn_info_from_declarator(fn_decl);
        // `= default` / `= delete` is the definition even though it has
        // no compound statement. `= 0` is not: the body, if any, is
        // elsewhere, and this declaration must stay a prototype so the
        // real body can absorb it.
        let has_body = node.child_by_field_name("body").is_some()
            || has_child_kind(node, "default_method_clause")
            || has_child_kind(node, "delete_method_clause");
        self.push_def(node, info, has_body);
    }

    fn push_def(&mut self, node: Node, info: FnInfo, has_body: bool) {
        if info.simple.is_empty() {
            return;
        }
        let qn = self.qualified_name(&info.qual_tail);
        let (sl, el) = line_range(node);
        let mut attributes = cuda_qualifiers(self.text(node));
        if is_virtual_method(node, self.source) {
            attributes.push("virtual".to_string());
        }
        self.facts.definitions.push(DefRecord {
            simple_name: info.simple,
            qualified_name: qn,
            variant: info.variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes,
            has_body,
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    /// Qualified name of `tail` under the current namespace/class stack.
    ///
    /// `tail` is either a bare identifier (`add`, inside the class) or
    /// the out-of-line declarator text (`Calc::add`, `math::Calc::add`).
    /// When the declarator already starts with the enclosing scope,
    /// prepending it again would produce `math::math::Calc::add`.
    fn qualified_name(&self, tail: &str) -> String {
        if self.scope.is_empty() {
            return tail.to_string();
        }
        let scope = self.scope.join("::");
        // A bare name equal to the class (`Kernel` inside `class Kernel`)
        // is a constructor, and its qualified name is `Kernel::Kernel`.
        // Only a declarator that already contains `::` can already include
        // the enclosing scope (`math::Calc::add` written inside `namespace math`).
        if tail.contains("::")
            && (tail == scope || tail.starts_with(&(scope.clone() + "::")))
        {
            tail.to_string()
        } else {
            format!("{scope}::{tail}")
        }
    }

    fn fn_info_from_declarator(&self, decl: Node) -> FnInfo {
        // function_declarator -> declarator: identifier | field_identifier |
        //   destructor_name | qualified_identifier. Pointer and reference
        //   declarators are unwrapped by the caller (`int *Robot::foo()`).
        if decl.kind() != "function_declarator" {
            return FnInfo::empty();
        }
        let Some(d) = decl.child_by_field_name("declarator") else {
            return FnInfo::empty();
        };
        match d.kind() {
            "identifier" => {
                let name = self.text(d).to_string();
                // If inside a class scope, it's a constructor if name == class name.
                let variant =
                    if self.scope.last().map(String::as_str) == Some(name.as_str()) {
                        DefVariant::Constructor
                    } else if self.scope.is_empty() {
                        DefVariant::FreeFunction
                    } else {
                        DefVariant::InherentMethod
                    };
                FnInfo::named(name, variant)
            }
            "field_identifier" => {
                let name = self.text(d).to_string();
                FnInfo::named(name, DefVariant::InherentMethod)
            }
            "destructor_name" => {
                // ~ClassName
                let name = format!("~{}", self.text(d).trim_start_matches('~'));
                FnInfo::named(name, DefVariant::Destructor)
            }
            "qualified_identifier" => self.qualified_fn_info(d),
            "operator_name" | "operator_cast" => {
                FnInfo::named(self.text(d).to_string(), DefVariant::InherentMethod)
            }
            _ => FnInfo::empty(),
        }
    }

    /// `void Robot::on_gcode_received()` and `int math::Calc::add()`.
    ///
    /// The simple name is the last segment (`on_gcode_received`, `add`),
    /// matching the header declaration inside the class. The qualified
    /// name is the declarator text (`Robot::on_gcode_received`), so the
    /// `(language, owner, method)` index the resolver looks up contains
    /// the body and not a second spelling of it.
    fn qualified_fn_info(&self, qual: Node) -> FnInfo {
        let leaf = deepest_name(qual);
        let simple = match leaf.kind() {
            "destructor_name" => {
                let t = self.text(leaf);
                if t.starts_with('~') {
                    t.to_string()
                } else {
                    format!("~{t}")
                }
            }
            _ => self.text(leaf).to_string(),
        };
        if simple.is_empty() {
            return FnInfo::empty();
        }
        let variant = if leaf.kind() == "destructor_name" || simple.starts_with('~') {
            DefVariant::Destructor
        } else if immediate_class(qual, self.source).as_deref() == Some(simple.as_str()) {
            DefVariant::Constructor
        } else {
            DefVariant::InherentMethod
        };
        FnInfo {
            simple,
            qual_tail: self.text(qual).to_string(),
            variant,
        }
    }

    /// `StreamOutput* streams` inside `class Kernel`. The call
    /// `kernel->streams->printf()` is in another file; only the type
    /// travels, via [`FileFacts::field_types`].
    fn record_field(&mut self, node: Node) {
        let Some(owner) = self.class_stack.last().cloned() else {
            return;
        };
        let Some(ty) = node
            .child_by_field_name("type")
            .and_then(|t| nominal_type(self.text(t)))
        else {
            return;
        };
        for declarator in declarator_fields(node) {
            let Some(field) = variable_name(declarator, self.source) else {
                continue;
            };
            self.facts.field_types.push(FieldType {
                owner: owner.clone(),
                field,
                type_name: ty.clone(),
            });
        }
    }

    /// `Kernel* kernel = new Kernel()` and other block/file declarations.
    /// A function prototype has no variable name and is skipped.
    fn record_local(&mut self, node: Node) {
        let Some(ty) = node
            .child_by_field_name("type")
            .and_then(|t| nominal_type(self.text(t)))
        else {
            return;
        };
        let scope_byte = node.start_byte() as u32;
        for declarator in declarator_fields(node) {
            let Some(var_name) = variable_name(declarator, self.source) else {
                continue;
            };
            self.facts.local_types.push(cgg_core::LocalType {
                var_name,
                type_name: ty.clone(),
                scope_byte,
            });
        }
    }

    fn record_type_alias(&mut self, name: String, target: String) {
        let target = target.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || target.is_empty() || name == target {
            return;
        }
        self.facts.type_aliases.push(MacroAlias {
            name,
            replacement: target,
        });
    }

    fn try_record_prototype(&mut self, node: Node) {
        // A macro in front of a method —
        // `JSON_HEDLEY_NON_NULL(2) token_type scan_literal(...) { ... }` —
        // makes the grammar's "declaration" swallow the whole body, and the
        // first call inside it (`JSON_ASSERT(...)`) then reads as the
        // declared name: a phantom method that made every real
        // `JSON_ASSERT` call in nlohmann/json ambiguous. The `{` check
        // below catches it. Rejecting every error-recovered declaration
        // instead is too broad: `void Printf(const char *f, ...)
        // FORMAT(1, 2);` recovers the same way and is a real prototype.
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if let Some(fn_decl) = unwrap_function_declarator(child) {
                // A prototype has no body: a `{` before its declarator
                // means this node spans a definition the grammar misread.
                if self.source[node.start_byte()..fn_decl.start_byte()].contains(&b'{') {
                    return;
                }
                let info = self.fn_info_from_declarator(fn_decl);
                // A declaration-only kernel (`__global__ void saxpy(...);`
                // in a header) is an entry point just as much as its
                // definition. `has_body` stays false so a later definition
                // can absorb this node.
                self.push_def(node, info, false);
                return;
            }
        }
    }

    fn record_function_macro(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }
        let qn = self.qn(&name);
        let (sl, el) = line_range(node);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
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
    }

    /// `#define THEKERNEL Kernel::instance` — kept as text so the type
    /// propagator can expand chains against class fields. A replacement
    /// that is empty or only punctuation is ignored.
    fn record_object_macro(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some(value_node) = node.child_by_field_name("value") else {
            return;
        };
        let replacement =
            unwrap_outer_parens(&collapse_ws(self.text(value_node))).to_string();
        if replacement.is_empty()
            || !replacement.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        {
            return;
        }
        self.facts
            .macro_aliases
            .push(MacroAlias { name, replacement });
    }

    fn record_include(&mut self, node: Node) {
        let Some(path_node) = node.child_by_field_name("path") else {
            return;
        };
        let kind = path_node.kind();
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
        } else if kind == "system_lib_string" {
            // A system include (`<gtest/gtest.h>`) names no file in the
            // tree, so it stays out of the `include` kind the cross-file
            // resolver follows. It is still the only evidence that a
            // library is in use, and framework detection needs it —
            // without this no C/C++ rule keyed on a system header could
            // ever fire. A distinct kind keeps the two consumers apart.
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

    /// `new Kernel()` is the call that enters `Kernel::Kernel`. The
    /// grammar does not treat it as a `call_expression`.
    fn record_new(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let Some(name) = nominal_type(self.text(ty)) else {
            return;
        };
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        // `(m->*table[i])(...)` often has no usable `function` field
        // because `->*` lands in an ERROR node. Scan the whole call.
        if let Some((name, recv)) = member_ptr_from_call_func(node, self.source) {
            self.facts.references.push(RefRecord {
                name,
                receiver_hint: recv,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
                ..Default::default()
            });
            return;
        }
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let func = unwrap_parens(func);
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "field_expression" => {
                let arg = func
                    .child_by_field_name("argument")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let field = func
                    .child_by_field_name("field")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // `this->Base::Close()` names the class explicitly: it is a
                // direct, non-virtual call to `Base::Close`. Record the
                // method as the name and the class as the receiver, the same
                // split as `Base::Close()` below. Keeping the qualified text
                // as the name only matched while out-of-line bodies carried
                // `Base::Close` as their simple name, which PR 8 fixed.
                match field.rfind("::") {
                    Some(pos) => (field[pos + 2..].to_string(), field[..pos].to_string()),
                    None => (field, arg),
                }
            }
            "qualified_identifier" => {
                let full = self.text(func);
                if let Some(pos) = full.rfind("::") {
                    let recv = full[..pos].to_string();
                    let name = full[pos + 2..].to_string();
                    (name, recv)
                } else {
                    (full.to_string(), String::new())
                }
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }

    /// `&Module::on_idle` inside a table initializer.
    fn record_member_ptr_take(&mut self, node: Node) {
        let Some(op) = node.child_by_field_name("operator") else {
            return;
        };
        if op.kind() != "&" {
            return;
        }
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        let Some((owner, method)) = owner_method_from_qual(self.text(arg)) else {
            return;
        };
        let Some(table) = enclosing_table_name(node, self.source) else {
            return;
        };
        self.facts.member_ptr_takes.push(MemberPtrTake {
            table,
            owner,
            method,
        });
    }
}

struct FnInfo {
    simple: String,
    /// Bare name, or the out-of-line declarator (`Calc::add`).
    qual_tail: String,
    variant: DefVariant,
}

impl FnInfo {
    fn empty() -> Self {
        Self {
            simple: String::new(),
            qual_tail: String::new(),
            variant: DefVariant::FreeFunction,
        }
    }

    fn named(name: String, variant: DefVariant) -> Self {
        Self {
            qual_tail: name.clone(),
            simple: name,
            variant,
        }
    }
}

/// `int *Robot::foo()` nests the function declarator inside a pointer
/// (or reference) declarator. Walk those wrappers down to it.
fn unwrap_function_declarator(node: Node) -> Option<Node> {
    let mut n = node;
    loop {
        if n.kind() == "function_declarator" {
            return Some(n);
        }
        if !matches!(
            n.kind(),
            "pointer_declarator"
                | "reference_declarator"
                | "attributed_declarator"
                | "parenthesized_declarator"
        ) {
            return None;
        }
        if let Some(inner) = n.child_by_field_name("declarator") {
            n = inner;
            continue;
        }
        let mut c = n.walk();
        let inner = n.children(&mut c).find(|ch| {
            matches!(
                ch.kind(),
                "function_declarator"
                    | "pointer_declarator"
                    | "reference_declarator"
                    | "attributed_declarator"
                    | "parenthesized_declarator"
            )
        })?;
        n = inner;
    }
}

fn has_child_kind(node: Node, kind: &str) -> bool {
    let mut c = node.walk();
    node.children(&mut c).any(|ch| ch.kind() == kind)
}

fn is_virtual_method(node: Node, source: &[u8]) -> bool {
    has_child_kind(node, "virtual")
        || has_child_kind(node, "pure_virtual_clause")
        || node
            .utf8_text(source)
            .ok()
            .is_some_and(|t| t.trim_start().starts_with("virtual "))
}

fn unwrap_parens(mut n: Node) -> Node {
    for _ in 0..8 {
        if n.kind() != "parenthesized_expression" {
            break;
        }
        let Some(inner) = n.named_child(0) else {
            break;
        };
        n = inner;
    }
    n
}

fn table_ident(node: Node, source: &[u8]) -> Option<String> {
    let n = unwrap_parens(node);
    match n.kind() {
        "identifier" => {
            let t = n.utf8_text(source).ok()?.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        "subscript_expression" => n
            .child_by_field_name("argument")
            .and_then(|a| table_ident(a, source))
            .or_else(|| n.named_child(0).and_then(|a| table_ident(a, source))),
        _ => None,
    }
}

fn member_ptr_from_call_func(func: Node<'_>, source: &[u8]) -> Option<(String, String)> {
    let table = scan_member_ptr_table(func, source)?;
    Some(("->*".to_string(), table))
}

/// `->*` is often inside an ERROR node rather than a `binary_expression`
/// when the callee is parenthesized: `(m->*table[i])(...)`.
fn scan_member_ptr_table(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut saw_op = false;
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "argument_list" {
            continue;
        }
        if n.kind() == "->*" || n.kind() == ".*" {
            saw_op = true;
            continue;
        }
        if saw_op && let Some(t) = table_ident(n, source) {
            return Some(t);
        }
        let mut kids = Vec::new();
        let mut c = n.walk();
        if c.goto_first_child() {
            loop {
                kids.push(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        // DFS left-to-right: push in reverse.
        for k in kids.into_iter().rev() {
            stack.push(k);
        }
    }
    None
}

fn owner_method_from_qual(raw: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    let (owner, method) = raw.rsplit_once("::")?;
    let owner = owner.rsplit("::").next()?.trim();
    let method = method.trim();
    if owner.is_empty()
        || method.is_empty()
        || !owner.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        || !method.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '~')
    {
        return None;
    }
    Some((owner.to_string(), method.to_string()))
}

fn enclosing_table_name(node: Node, source: &[u8]) -> Option<String> {
    let mut n = node;
    for _ in 0..16 {
        n = n.parent()?;
        if matches!(
            n.kind(),
            "init_declarator" | "declaration" | "field_declaration"
        ) {
            for d in declarator_fields(n) {
                if let Some(name) = variable_name(d, source) {
                    return Some(name);
                }
            }
            if let Some(d) = n.child_by_field_name("declarator")
                && let Some(name) = variable_name(d, source)
            {
                return Some(name);
            }
        }
    }
    None
}

fn declarator_fields(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return Vec::new();
    }
    let mut out = Vec::new();
    loop {
        if cursor.field_name() == Some("declarator") {
            out.push(cursor.node());
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    out
}

/// Collapse runs of whitespace in a preprocessor argument to a single
/// space so `#define THEKERNEL Kernel::instance` and a multiline form
/// share one replacement key.
fn collapse_ws(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `#define THE_APP (App::instance)` stores `App::instance`. Only the
/// matching outer pair is stripped; `(a) + (b)` is left alone.
fn unwrap_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return if i + ch.len_utf8() == s.len() {
                        s[1..i].trim()
                    } else {
                        s
                    };
                }
            }
            _ => {}
        }
    }
    s
}

/// Identifier a declaration names, or `None` when the declarator is a
/// function (a prototype, not a variable or field).
fn variable_name(node: Node, source: &[u8]) -> Option<String> {
    let mut n = node;
    loop {
        match n.kind() {
            "identifier" | "field_identifier" => {
                let text = n.utf8_text(source).unwrap_or("").trim();
                if text.is_empty() {
                    return None;
                }
                return Some(text.to_string());
            }
            "function_declarator" => return None,
            "init_declarator"
            | "pointer_declarator"
            | "array_declarator"
            | "reference_declarator"
            | "attributed_declarator"
            | "parenthesized_declarator" => {
                n = inner_declarator(n)?;
            }
            _ => return None,
        }
    }
}

fn inner_declarator(node: Node) -> Option<Node> {
    if let Some(inner) = node.child_by_field_name("declarator") {
        return Some(inner);
    }
    let mut c = node.walk();
    node.children(&mut c).find(|ch| {
        matches!(
            ch.kind(),
            "identifier"
                | "field_identifier"
                | "function_declarator"
                | "pointer_declarator"
                | "reference_declarator"
                | "array_declarator"
                | "init_declarator"
                | "parenthesized_declarator"
                | "attributed_declarator"
                | "qualified_identifier"
        )
    })
}

/// `const StreamOutput*` / `std::string` → `StreamOutput` / `string`,
/// dropping a result that is not a nominal type (`int`, `void`).
///
/// `std::unique_ptr<Kernel>` and `shared_ptr<const StreamOutput>` peel
/// to the pointed-to type: the field hop needs `Kernel`, not the wrapper.
fn nominal_type(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    loop {
        let next = s.trim_start();
        if let Some(rest) = next
            .strip_prefix("const ")
            .or_else(|| next.strip_prefix("volatile "))
            .or_else(|| next.strip_prefix("typename "))
        {
            s = rest;
        } else {
            s = next;
            break;
        }
    }
    let s = s.trim_matches(|c: char| c == '*' || c == '&' || c.is_whitespace());
    let s = s.rsplit("::").next().unwrap_or(s).trim();
    if let Some(arg) = first_template_arg(s)
        && let Some(inner) = nominal_type(arg)
    {
        return Some(inner);
    }
    let s = s.split('<').next().unwrap_or(s).trim();
    let s = s.trim_matches(|c: char| c == '*' || c == '&');
    if s.is_empty() || !s.starts_with(|c: char| c.is_uppercase()) {
        return None;
    }
    Some(s.to_string())
}

/// Rightmost `name` field. `math::Calc::add` nests qualified identifiers,
/// and `child_by_field_name` returns the first, which is not the method.
fn last_name_field(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    let mut last = None;
    loop {
        if cursor.field_name() == Some("name") {
            last = Some(cursor.node());
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    last
}

fn deepest_name(node: Node) -> Node {
    let mut n = node;
    while n.kind() == "qualified_identifier" {
        let Some(name) = last_name_field(n) else {
            break;
        };
        n = name;
    }
    n
}

/// Class name immediately qualifying the method: `Calc` in
/// `math::Calc::add`, `Calc` in `Calc::~Calc`.
fn immediate_class(qual: Node, source: &[u8]) -> Option<String> {
    let mut n = qual;
    loop {
        if n.kind() != "qualified_identifier" {
            return None;
        }
        let name = last_name_field(n)?;
        if name.kind() == "qualified_identifier" {
            n = name;
            continue;
        }
        let scope = n.child_by_field_name("scope")?;
        let text = scope.utf8_text(source).unwrap_or("");
        let last = text.rsplit("::").next().unwrap_or(text);
        if last.is_empty() {
            return None;
        }
        return Some(last.to_string());
    }
}

/// Drop a C-family prototype when the tree also contains its body.
///
/// Header `void Robot::on_gcode_received(void*);` and `Robot.cpp`'s
/// definition are one function, and so are a `.h` parsed as C and the
/// `.c` or `.m` file that defines it. Keeping both makes every resolved
/// call land on the prototype, which has no outgoing edges, while the
/// body — which has them — has no callers. A definition counts as a
/// body only when `has_body` is set. A prototype is removed when any
/// body shares its qualified name and parameter list.
/// Two overloads stay apart because the parameter lists differ. Two
/// bodies of one signature (a test stub beside the real definition)
/// both stay; only the prototype goes. Anonymous-namespace definitions
/// are per translation unit, so a body there absorbs a prototype only
/// in the same file.
pub fn unify_declarations(files: &mut [&mut FileFacts]) {
    struct Body {
        file: usize,
        def_idx: usize,
        key: String,
    }
    let mut bodies: std::collections::HashMap<String, Vec<Body>> =
        std::collections::HashMap::new();
    for (file_idx, facts) in files.iter().enumerate() {
        if !cgg_core::same_family(&facts.language, "c") {
            continue;
        }
        for (def_idx, def) in facts.definitions.iter().enumerate() {
            if !def.has_body {
                continue;
            }
            let Some(key) = overload_key(&def.signature_hint) else {
                continue;
            };
            bodies
                .entry(strip_template_args(&def.qualified_name))
                .or_default()
                .push(Body {
                    file: file_idx,
                    def_idx,
                    key,
                });
        }
    }
    let mut drop_at: Vec<(usize, usize)> = Vec::new();
    let mut unified: Vec<(usize, String, String)> = Vec::new();
    let mut merges: Vec<(usize, usize, bool, Vec<String>)> = Vec::new();
    for (file_idx, facts) in files.iter().enumerate() {
        if !cgg_core::same_family(&facts.language, "c") {
            continue;
        }
        for (def_idx, def) in facts.definitions.iter().enumerate() {
            if def.has_body || def.attributes.iter().any(|a| a == "macro") {
                continue;
            }
            // `UnwindCursor::getReg` (the in-class prototype) and
            // `UnwindCursor<A, R>::getReg` (the out-of-line body) are one
            // member: the template arguments are not part of its identity.
            let Some(candidates) = bodies.get(&strip_template_args(&def.qualified_name))
            else {
                continue;
            };
            let Some(key) = overload_key(&def.signature_hint) else {
                continue;
            };
            // Every body of this overload, restricted to this file when
            // the name is an anonymous-namespace function (each
            // translation unit has its own). One body or several — a
            // test stub next to the real definition — the prototype is
            // not an extra function. Zero bodies: keep the declaration.
            let mut hits: Vec<_> = candidates.iter().filter(|b| b.key == key).collect();
            if def.qualified_name.contains("(anonymous)") {
                hits.retain(|b| b.file == file_idx);
            }
            if hits.is_empty() {
                continue;
            }
            // Bodies in more than one other file: independent translation
            // units that happen to share a name (cuda-samples defines
            // `BlackScholesCPU` once per sample). The declaration is the
            // only thing that says which one this file means, so keep it
            // rather than merging it into all of them.
            let other_files: std::collections::BTreeSet<usize> = hits
                .iter()
                .map(|h| h.file)
                .filter(|f| *f != file_idx)
                .collect();
            // Only for free functions: a class member defined in two files is
            // one function with a test stub beside it (the typed call must
            // reach both), not two unrelated programs.
            if other_files.len() > 1
                && !hits.iter().any(|h| h.file == file_idx)
                && def.variant == DefVariant::FreeFunction
            {
                continue;
            }
            let virt = def.attributes.iter().any(|a| a == "virtual");
            let bases = def.base_types.clone();
            for h in &hits {
                merges.push((h.file, h.def_idx, virt, bases.clone()));
            }
            // The body may live in a file the caller never includes; keep
            // the header's claim to the name so `#include` still reaches it.
            if hits.iter().any(|h| h.file != file_idx) {
                unified.push((
                    file_idx,
                    def.simple_name.clone(),
                    def.qualified_name.clone(),
                ));
            }
            drop_at.push((file_idx, def_idx));
        }
    }
    for (file, def_idx, virt, bases) in merges {
        let def = &mut files[file].definitions[def_idx];
        if virt && !def.attributes.iter().any(|a| a == "virtual") {
            def.attributes.push("virtual".into());
        }
        for b in bases {
            if !def.base_types.contains(&b) {
                def.base_types.push(b);
            }
        }
    }
    for (file, simple, qualified) in unified {
        let decls = &mut files[file].unified_decls;
        if !decls.iter().any(|(s, q)| *s == simple && *q == qualified) {
            decls.push((simple, qualified));
        }
    }
    let drop_set: std::collections::HashSet<(usize, usize)> =
        drop_at.into_iter().collect();
    for (file_idx, facts) in files.iter_mut().enumerate() {
        if !cgg_core::same_family(&facts.language, "c") {
            continue;
        }
        let mut i = 0usize;
        facts.definitions.retain(|_| {
            let keep = !drop_set.contains(&(file_idx, i));
            i += 1;
            keep
        });
    }
}

/// A qualified name with every `<...>` argument list removed:
/// `ns::Cursor<A, R>::get` → `ns::Cursor::get`.
fn strip_template_args(qn: &str) -> String {
    let mut out = String::with_capacity(qn.len());
    let mut depth = 0u32;
    for ch in qn.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Parameter list plus cv/ref qualifiers, with names and default
/// arguments removed, so `void add(int a, int b);` and
/// `int Calc::add(int a, int b)` name one overload.
fn overload_key(sig: &str) -> Option<String> {
    // Definitions often carry the parameter comments the header omits
    // (`BYTE drv, /* Logical drive number */ BYTE sfd`). Those comments
    // are not part of the overload.
    let sig = strip_comments(sig);
    let open = sig.find('(')?;
    let close = matching_paren(&sig, open)?;
    let mut parts: Vec<String> = split_params(&sig[open + 1..close])
        .into_iter()
        .map(|p| normalize_param(&p))
        .collect();
    // `void f(void)` is the same function as `void f()`.
    if parts.len() == 1 && parts[0] == "void" {
        parts.clear();
    }
    let quals = method_qualifiers(&sig[close + 1..]);
    Some(format!("{quals}|{}", parts.join("|")))
}

fn matching_paren(sig: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in sig[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_params(params: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, ch) in params.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = params[start..i].trim();
                if !part.is_empty() {
                    out.push(part.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = params[start..].trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    out
}

fn normalize_param(param: &str) -> String {
    let param = strip_default(param);
    let collapsed = param.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut owned: Vec<String> = collapsed
        .split(' ')
        .filter(|t| !t.is_empty())
        .map(strip_array_name)
        .collect();
    if let Some(last) = owned.last().map(String::as_str) {
        let stripped = strip_glued_name(last).to_string();
        if stripped != last {
            owned.pop();
            if !stripped.is_empty() {
                owned.push(stripped);
            }
        } else if should_drop_trailing_name(&owned) {
            owned.pop();
        }
    }
    let joined = collapse_pointer_space(&owned.join(" ")).replace(" [", "[");
    // `std::string` in a header and `string` in a .cpp that has
    // `using std::string` are the same parameter. `T[]` and `T*` are
    // the same function parameter type.
    decay_std_and_array(&joined)
}

fn decay_std_and_array(param: &str) -> String {
    let param = param.replace("std::", "");
    let Some(bracket) = param.rfind('[') else {
        return param;
    };
    if !param.ends_with(']') {
        return param;
    }
    format!("{}*", param[..bracket].trim_end())
}

/// `float xs[]` and `float[]` are the same parameter. The name sits
/// immediately before the brackets.
fn strip_array_name(token: &str) -> String {
    let Some(bracket) = token.find('[') else {
        return token.to_string();
    };
    let name = &token[..bracket];
    if !name.is_empty() && is_param_name(name) {
        token[bracket..].to_string()
    } else {
        token.to_string()
    }
}

/// Drop a trailing identifier when it is a parameter name.
///
/// `enum MOTION_MODE_T motion_mode` drops `motion_mode`.
/// `enum MOTION_MODE_T` does not drop `MOTION_MODE_T`: the token before
/// it is the introducer, so that identifier is the type.
/// `const uint32_t` / `volatile Foo` are the same: after a cv-qualifier
/// the identifier is the type, or a header that omitted the name would
/// not match `const uint32_t n` in the body.
fn should_drop_trailing_name(tokens: &[String]) -> bool {
    if tokens.len() < 2 {
        return false;
    }
    let last = tokens.last().map(String::as_str).unwrap_or("");
    if !is_param_name(last) {
        return false;
    }
    let prev = tokens[tokens.len() - 2].as_str();
    !matches!(
        prev,
        "enum" | "struct" | "class" | "union" | "typename" | "const" | "volatile"
    )
}

fn strip_comments(sig: &str) -> String {
    let mut out = String::with_capacity(sig.len());
    let mut chars = sig.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(d) = chars.next() {
                if d == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            out.push(' ');
        } else if c == '/' && chars.peek() == Some(&'/') {
            for d in chars.by_ref() {
                if d == '\n' {
                    break;
                }
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

fn strip_default(param: &str) -> &str {
    let mut depth = 0i32;
    for (i, ch) in param.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => return param[..i].trim(),
            _ => {}
        }
    }
    param.trim()
}

/// `*stream` / `&name` / `&&name` → the pointer or reference tokens.
fn strip_glued_name(token: &str) -> &str {
    let rest = token.trim_start_matches(['*', '&']);
    if rest.len() < token.len() && is_param_name(rest) {
        &token[..token.len() - rest.len()]
    } else {
        token
    }
}

fn is_param_name(token: &str) -> bool {
    let mut chars = token.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    if !chars.all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return false;
    }
    !is_type_keyword(token)
}

fn is_type_keyword(token: &str) -> bool {
    matches!(
        token,
        "int"
            | "void"
            | "char"
            | "bool"
            | "float"
            | "double"
            | "long"
            | "short"
            | "signed"
            | "unsigned"
            | "auto"
            | "const"
            | "volatile"
            | "wchar_t"
            | "char8_t"
            | "char16_t"
            | "char32_t"
            | "size_t"
    )
}

fn collapse_pointer_space(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let prev_ptr = out.ends_with(['*', '&']);
            let next_ptr = chars.get(j).is_some_and(|c| *c == '*' || *c == '&');
            if !prev_ptr && !next_ptr {
                out.push(' ');
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn method_qualifiers(after: &str) -> String {
    let mut rest = after;
    if let Some(i) = rest.find(['{', ';']) {
        rest = &rest[..i];
    }
    let rest = strip_call(rest, "noexcept");
    let rest = strip_call(&rest, "throw");
    let rest = rest.split('=').next().unwrap_or(&rest);
    // Glued forms (`const&`, `const&&`) are one token, so this is a
    // substring check rather than a token match. `noexcept(...)` was
    // already removed; a `const` inside it must not count.
    let mut flags = Vec::new();
    if rest.contains("const") {
        flags.push("const");
    }
    if rest.contains("volatile") {
        flags.push("volatile");
    }
    if rest.contains("&&") {
        flags.push("&&");
    } else if rest.contains('&') {
        flags.push("&");
    }
    flags.join(" ")
}

fn strip_call(s: &str, kw: &str) -> String {
    let Some(at) = s.find(kw) else {
        return s.to_string();
    };
    let after = s[at + kw.len()..].trim_start();
    if !after.starts_with('(') {
        return s.to_string();
    }
    let Some(rel) = matching_paren(after, 0) else {
        return s.to_string();
    };
    let mut out = String::new();
    out.push_str(&s[..at]);
    out.push_str(&after[rel + 1..]);
    out
}

fn line_range(n: Node) -> (u32, u32) {
    (
        (n.start_position().row as u32) + 1,
        (n.end_position().row as u32) + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        extract_at(src, "/tmp/__cgg_test__/x.cpp")
    }

    fn extract_at(src: &str, path: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_cpp::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        CppPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from(path),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn namespace_qualified_names() {
        let src = "namespace math { namespace detail { void compute() {} } }\n";
        let f = extract(src);
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name == "math::detail::compute"),
            "got: {:?}",
            f.definitions
                .iter()
                .map(|d| &d.qualified_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn class_method_and_constructor() {
        let src = r#"
namespace ns {
class Foo {
public:
    Foo() {}
    ~Foo() {}
    int bar(int x) { return x; }
};
}
"#;
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"ns::Foo::Foo"), "got: {qns:?}");
        // A constructor in a global class is `Foo::Foo`, not bare `Foo`.
        // The bare name is what `new Foo()` looks up.
        let global = extract("class Foo { Foo(); };\n");
        let ctor = global
            .definitions
            .iter()
            .find(|d| d.simple_name == "Foo")
            .unwrap();
        assert_eq!(ctor.qualified_name, "Foo::Foo");
        assert!(qns.contains(&"ns::Foo::~Foo"), "got: {qns:?}");
        assert!(qns.contains(&"ns::Foo::bar"), "got: {qns:?}");
        let ctor = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "Foo")
            .unwrap();
        assert_eq!(ctor.variant, DefVariant::Constructor);
        let dtor = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "~Foo")
            .unwrap();
        assert_eq!(dtor.variant, DefVariant::Destructor);
    }

    #[test]
    fn qualified_call_expression() {
        let src = "void f() { math::detail::compute(); }\n";
        let f = extract(src);
        let r = f.references.iter().find(|r| r.name == "compute").unwrap();
        assert_eq!(r.receiver_hint, "math::detail");
    }

    #[test]
    fn a_macro_prefixed_method_body_mints_no_phantom_prototype() {
        let f = extract(
            "#define JSON_ASSERT(x) assert(x)\n#define NON_NULL(n)\nclass lexer {\n    NON_NULL(2)\n    int scan(const char* t)\n    {\n        JSON_ASSERT(t != nullptr);\n        return 0;\n    }\n};\n",
        );
        let phantoms: Vec<_> = f
            .definitions
            .iter()
            .filter(|d| {
                d.simple_name == "JSON_ASSERT"
                    && !d.attributes.iter().any(|a| a == "macro")
            })
            .map(|d| d.qualified_name.clone())
            .collect();
        assert!(phantoms.is_empty(), "phantom prototypes: {phantoms:?}");
    }

    #[test]
    fn qualified_member_call_names_the_method_and_its_class() {
        let f = extract(
            "struct Base { bool Close(); };\nstruct D : Base { bool Close(); };\nbool D::Close() { return this->Base::Close(); }\n",
        );
        let r = f
            .references
            .iter()
            .find(|r| r.name == "Close")
            .expect("`this->Base::Close()` is a call to `Close`");
        assert_eq!(r.receiver_hint, "Base");
    }

    #[test]
    fn field_expression_call() {
        let src = "void f() { obj.run(); ptr->exec(); }\n";
        let f = extract(src);
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("run", "obj")), "got: {refs:?}");
        assert!(refs.contains(&("exec", "ptr")), "got: {refs:?}");
    }

    #[test]
    fn out_of_line_simple_name_is_the_method() {
        let f = extract(
            "void Robot::on_gcode_received(void *argument) { (void)argument; }\n",
        );
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "on_gcode_received")
            .expect("simple name is the method, not Robot::on_gcode_received");
        assert_eq!(d.qualified_name, "Robot::on_gcode_received");
        assert!(d.has_body);
    }

    #[test]
    fn out_of_line_inside_namespace_matches_the_class() {
        let header = extract(
            "namespace math {\nclass Calc {\npublic:\n    static int add(int a, int b);\n};\n}\n",
        );
        let decl = header
            .definitions
            .iter()
            .find(|d| d.simple_name == "add")
            .unwrap();
        assert_eq!(decl.qualified_name, "math::Calc::add");
        assert!(!decl.has_body);

        let body = extract(
            "namespace math {\nint Calc::add(int a, int b) { return a + b; }\n}\n",
        );
        let def = body
            .definitions
            .iter()
            .find(|d| d.simple_name == "add")
            .unwrap();
        assert_eq!(def.qualified_name, "math::Calc::add");
        assert!(def.has_body);

        // The declarator is already fully qualified; the enclosing
        // namespace must not be prepended a second time.
        let full = extract(
            "namespace math {\nint math::Calc::add(int a, int b) { return a + b; }\n}\n",
        );
        let def = full
            .definitions
            .iter()
            .find(|d| d.simple_name == "add")
            .unwrap();
        assert_eq!(def.qualified_name, "math::Calc::add");
    }

    #[test]
    fn out_of_line_constructor_destructor_and_pointer_return() {
        let f = extract(
            "Calc::~Calc() {}\nCalc::Calc(int a) : x(a) {}\nint *Calc::foo() { return 0; }\n",
        );
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Calc::~Calc"), "got: {qns:?}");
        assert!(qns.contains(&"Calc::Calc"), "got: {qns:?}");
        assert!(qns.contains(&"Calc::foo"), "got: {qns:?}");
        let ctor = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Calc::Calc")
            .unwrap();
        assert_eq!(ctor.variant, DefVariant::Constructor);
        assert_eq!(ctor.simple_name, "Calc");
        let dtor = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "~Calc")
            .unwrap();
        assert_eq!(dtor.variant, DefVariant::Destructor);
        let foo = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "foo")
            .unwrap();
        assert!(foo.has_body);
    }

    #[test]
    fn function_pointer_field_is_not_a_callable() {
        let f = extract("class A { void (*cb)(); int x; };\n");
        assert!(
            f.definitions.iter().all(|d| d.simple_name != "cb"),
            "function pointer field recorded as a callable: {:?}",
            f.definitions
                .iter()
                .map(|d| &d.qualified_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn local_variable_and_field_types_are_recorded() {
        let f = extract(
            r#"
class Kernel {
    StreamOutput* streams;
    void add_module(Module* module);
};
void init() {
    Kernel* kernel = new Kernel();
    kernel->add_module(kernel);
    kernel->streams->printf();
}
"#,
        );
        let local = f
            .local_types
            .iter()
            .find(|t| t.var_name == "kernel")
            .expect("kernel local");
        assert_eq!(local.type_name, "Kernel");
        let field = f
            .field_types
            .iter()
            .find(|t| t.owner == "Kernel" && t.field == "streams")
            .expect("streams field");
        assert_eq!(field.type_name, "StreamOutput");
        let wrapped = extract(
            "class Kernel { std::unique_ptr<StreamOutput> out; shared_ptr<const Robot> robot; };\n",
        );
        let out = wrapped
            .field_types
            .iter()
            .find(|t| t.field == "out")
            .expect("unique_ptr field");
        assert_eq!(out.type_name, "StreamOutput");
        let robot = wrapped
            .field_types
            .iter()
            .find(|t| t.field == "robot")
            .expect("shared_ptr field");
        assert_eq!(robot.type_name, "Robot");
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("add_module", "kernel")), "refs: {refs:?}");
        assert!(
            refs.contains(&("printf", "kernel->streams")),
            "refs: {refs:?}"
        );
    }

    #[test]
    fn object_like_macros_and_static_fields_are_recorded() {
        let f = extract(
            r#"
#define THEKERNEL Kernel::instance
#define THECONVEYOR THEKERNEL->conveyor
#define THEROBOT THEKERNEL->robot
#define MAX_WCS 9UL
class Kernel {
public:
    static Kernel* instance;
    Robot* robot;
    Conveyor* conveyor;
};
"#,
        );
        let aliases: Vec<(&str, &str)> = f
            .macro_aliases
            .iter()
            .map(|m| (m.name.as_str(), m.replacement.as_str()))
            .collect();
        assert!(
            aliases.contains(&("THEKERNEL", "Kernel::instance")),
            "aliases: {aliases:?}"
        );
        let wrapped = extract("#define THE_APP (App::instance)\n");
        assert!(
            wrapped
                .macro_aliases
                .iter()
                .any(|m| m.name == "THE_APP" && m.replacement == "App::instance"),
            "parenthesized replacement: {:?}",
            wrapped.macro_aliases
        );
        assert!(
            aliases.contains(&("THECONVEYOR", "THEKERNEL->conveyor")),
            "aliases: {aliases:?}"
        );
        assert!(
            aliases.contains(&("THEROBOT", "THEKERNEL->robot")),
            "aliases: {aliases:?}"
        );
        assert!(
            aliases.iter().all(|(n, _)| *n != "MAX_WCS"),
            "numeric macro should be ignored: {aliases:?}"
        );
        let instance = f
            .field_types
            .iter()
            .find(|t| t.owner == "Kernel" && t.field == "instance")
            .expect("instance field");
        assert_eq!(instance.type_name, "Kernel");
        let robot = f
            .field_types
            .iter()
            .find(|t| t.owner == "Kernel" && t.field == "robot")
            .expect("robot field");
        assert_eq!(robot.type_name, "Robot");
    }

    #[test]
    fn bases_and_virtual_are_recorded_on_methods() {
        let f = extract(
            r#"
class Shape { public: virtual void draw(); };
class Circle : public Shape { public: void draw(); };
"#,
        );
        let shape = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Shape::draw")
            .expect("Shape::draw");
        assert!(
            shape.attributes.iter().any(|a| a == "virtual"),
            "attrs: {:?}",
            shape.attributes
        );
        let circle = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Circle::draw")
            .expect("Circle::draw");
        assert!(
            circle.base_types.iter().any(|b| b == "Shape"),
            "bases: {:?}",
            circle.base_types
        );
        let virt = extract(
            "class Shape {};\nclass Diamond : virtual public Shape { public: void draw(); };\n",
        );
        let draw = virt
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Diamond::draw")
            .expect("Diamond::draw");
        assert!(
            draw.base_types.iter().any(|b| b == "Shape"),
            "virtual public base: {:?}",
            draw.base_types
        );
    }

    #[test]
    fn member_pointer_table_and_indexed_call_are_recorded() {
        let f = extract(
            r#"
class Module {
public:
    virtual void on_idle(void*);
};
typedef void (Module::*CB)(void*);
const CB table[] = { &Module::on_idle };
void call_event(Module* m, int i) { (m->*table[i])(0); }
"#,
        );
        assert!(
            f.member_ptr_takes.iter().any(|t| {
                t.table == "table" && t.owner == "Module" && t.method == "on_idle"
            }),
            "takes: {:?}",
            f.member_ptr_takes
        );
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("->*", "table")), "refs: {refs:?}");
    }

    #[test]
    fn unify_copies_virtual_and_bases_onto_the_body() {
        let mut header = extract_at(
            "class Shape { public: virtual void draw(); };\nclass Circle : public Shape { public: void draw(); };\n",
            "/tmp/__cgg_test__/shape.h",
        );
        let mut body =
            extract_at("void Circle::draw() {}\n", "/tmp/__cgg_test__/shape.cpp");
        unify_declarations(&mut [&mut header, &mut body]);
        let circle = body
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Circle::draw")
            .expect("body");
        assert!(
            circle.base_types.iter().any(|b| b == "Shape"),
            "bases: {:?}",
            circle.base_types
        );
    }

    #[test]
    fn anonymous_namespace_is_its_own_scope() {
        let f = extract("namespace { void helper() {} }\n");
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "helper")
            .unwrap();
        assert_eq!(d.qualified_name, "(anonymous)::helper");
    }

    #[test]
    fn prototype_is_dropped_when_the_body_is_in_another_file() {
        let mut header = extract_at(
            "namespace math {\nclass Calc {\npublic:\n    static int add(int a, int b);\n    int* foo();\n    void set(int x = 0);\n    bool idle() const;\n};\n}\n",
            "/tmp/__cgg_test__/math.hpp",
        );
        let mut body = extract_at(
            "namespace math {\nint Calc::add(int a, int b) { return a + b; }\nint* Calc::foo() { return 0; }\nvoid Calc::set(int x) {}\nbool Calc::idle() const { return true; }\n}\n",
            "/tmp/__cgg_test__/math.cpp",
        );
        unify_declarations(&mut [&mut header, &mut body]);
        assert!(
            header.definitions.iter().all(|d| d.simple_name != "add"
                && d.simple_name != "foo"
                && d.simple_name != "set"
                && d.simple_name != "idle"),
            "prototypes should be absorbed, still have: {:?}",
            header
                .definitions
                .iter()
                .map(|d| &d.qualified_name)
                .collect::<Vec<_>>()
        );
        for name in ["add", "foo", "set", "idle"] {
            assert!(
                body.definitions
                    .iter()
                    .any(|d| d.simple_name == name && d.has_body),
                "missing body {name}"
            );
        }
    }

    #[test]
    fn enum_comments_and_array_parameters_still_match() {
        let mut header = extract_at(
            "class Robot {\n  void process_move(Gcode *gcode, enum MOTION_MODE_T);\n  void go(const float[]);\n};\n",
            "/tmp/__cgg_test__/Robot.h",
        );
        let mut body = extract_at(
            "void Robot::process_move(Gcode *gcode, /* mode */ enum MOTION_MODE_T motion_mode) {}\nvoid Robot::go(const float cartesian_mm[]) {}\n",
            "/tmp/__cgg_test__/Robot.cpp",
        );
        unify_declarations(&mut [&mut header, &mut body]);
        assert!(
            header.definitions.is_empty(),
            "prototypes left: {:?}",
            header
                .definitions
                .iter()
                .map(|d| &d.qualified_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn cv_qualified_unnamed_type_matches_the_named_body() {
        // `const uint32_t` is the type, not a name to drop. A header that
        // omits the parameter name used to key as `const` and miss the body.
        let mut header = extract_at(
            "class Widget {\n  void draw(const uint32_t);\n  void paint(volatile Foo);\n};\n",
            "/tmp/__cgg_test__/Widget.h",
        );
        let mut body = extract_at(
            "void Widget::draw(const uint32_t n) {}\nvoid Widget::paint(volatile Foo x) {}\n",
            "/tmp/__cgg_test__/Widget.cpp",
        );
        unify_declarations(&mut [&mut header, &mut body]);
        assert!(
            header.definitions.is_empty(),
            "left: {:?}",
            header
                .definitions
                .iter()
                .map(|d| (&d.qualified_name, &d.signature_hint))
                .collect::<Vec<_>>()
        );
        assert!(
            body.definitions
                .iter()
                .any(|d| d.simple_name == "draw" && d.has_body)
        );
        assert!(
            body.definitions
                .iter()
                .any(|d| d.simple_name == "paint" && d.has_body)
        );
    }

    #[test]
    fn std_qualification_and_array_parameters_match_the_body() {
        let mut header = extract_at(
            "class ConfigSource {\n  bool process_line(const std::string &buffer);\n  void update(const unsigned char *buf, size_t n);\n};\n",
            "/tmp/__cgg_test__/ConfigSource.h",
        );
        let mut body = extract_at(
            "bool ConfigSource::process_line(const string &buffer) { return true; }\nvoid ConfigSource::update(const unsigned char input[], size_t length) {}\n",
            "/tmp/__cgg_test__/ConfigSource.cpp",
        );
        unify_declarations(&mut [&mut header, &mut body]);
        assert!(
            header.definitions.is_empty(),
            "left: {:?}",
            header
                .definitions
                .iter()
                .map(|d| (&d.qualified_name, &d.signature_hint))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn overloads_and_anonymous_bodies_are_not_merged_away() {
        let mut header = extract_at(
            "void f(int x);\nvoid f(int x, int y);\nvoid helper();\n",
            "/tmp/__cgg_test__/a.hpp",
        );
        let mut body = extract_at(
            "void f(int x) {}\nvoid f(double x) {}\nnamespace { void helper() {} }\n",
            "/tmp/__cgg_test__/a.cpp",
        );
        unify_declarations(&mut [&mut header, &mut body]);
        // `f(int)` has a body; `f(int, int)` does not. `f(double)` is a
        // different overload and must not absorb `f(int, int)`.
        let header_names: Vec<&str> = header
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(
            !header_names.contains(&"f")
                || header
                    .definitions
                    .iter()
                    .filter(|d| d.qualified_name == "f")
                    .count()
                    == 1,
            "got header: {header_names:?}"
        );
        assert_eq!(
            header
                .definitions
                .iter()
                .filter(|d| d.qualified_name == "f")
                .count(),
            1,
            "only the unmatched overload stays, got {header_names:?}"
        );
        assert!(
            header
                .definitions
                .iter()
                .any(|d| d.qualified_name == "helper")
        );
        assert!(
            body.definitions
                .iter()
                .any(|d| d.qualified_name == "(anonymous)::helper")
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

    fn extract_c(file: u32, src: &str, path: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        super::super::c::CPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(file),
            &PathBuf::from(path),
            &tree,
            src.as_bytes(),
        )
    }

    /// A C header prototype is dropped when a C body file defines the
    /// same function. Runs the C plugin: unification only sees a body
    /// when that plugin sets `has_body`.
    #[test]
    fn c_header_prototype_is_unified_with_c_body() {
        let mut header = extract_c(
            0,
            "size_t qlz_decompress(const char *src, void *dst);\n",
            "/tmp/__cgg_test__/quicklz.h",
        );
        let mut body = extract_c(
            1,
            "size_t qlz_decompress(const char *source, void *destination) { return 0; }\n",
            "/tmp/__cgg_test__/quicklz.c",
        );
        assert!(
            header
                .definitions
                .iter()
                .any(|d| d.simple_name == "qlz_decompress" && !d.has_body),
            "C prototype must be extracted with has_body false"
        );
        assert!(
            body.definitions
                .iter()
                .any(|d| d.simple_name == "qlz_decompress" && d.has_body),
            "C function_definition must be extracted with has_body true"
        );
        unify_declarations(&mut [&mut header, &mut body]);
        assert!(
            header.definitions.is_empty(),
            "C header prototype should be absorbed by C body; left: {:?}",
            header
                .definitions
                .iter()
                .map(|d| &d.qualified_name)
                .collect::<Vec<_>>()
        );
        assert!(
            header
                .unified_decls
                .iter()
                .any(|(s, _)| s == "qlz_decompress"),
            "header must keep the name so #include still reaches the body: {:?}",
            header.unified_decls
        );
        assert!(
            body.definitions
                .iter()
                .any(|d| d.simple_name == "qlz_decompress" && d.has_body),
            "C body should keep its definition"
        );
    }

    /// The body may be compiled as C++. The header is still the C plugin.
    #[test]
    fn c_header_prototype_is_unified_with_cpp_body() {
        let mut header = extract_c(
            0,
            "size_t qlz_decompress(const char *src, void *dst);\n",
            "/tmp/__cgg_test__/quicklz.h",
        );
        let mut body = extract_at(
            "size_t qlz_decompress(const char *source, void *destination) { return 0; }\n",
            "/tmp/__cgg_test__/quicklz.cpp",
        );
        assert_eq!(body.language, "cpp");
        unify_declarations(&mut [&mut header, &mut body]);
        assert!(
            header.definitions.is_empty(),
            "C prototype should be absorbed by the C++ body; left: {:?}",
            header
                .definitions
                .iter()
                .map(|d| &d.qualified_name)
                .collect::<Vec<_>>()
        );
        assert!(
            header
                .unified_decls
                .iter()
                .any(|(s, _)| s == "qlz_decompress")
        );
        assert!(
            body.definitions
                .iter()
                .any(|d| d.has_body && d.simple_name == "qlz_decompress")
        );
    }
}

/// CUDA execution-space qualifiers, as attributes.
///
/// §8 of the design: `tree-sitter-cpp` parses `saxpy<<<a,b>>>(args)` as
/// nested comparison operators, so a kernel launch produces no edge at
/// all and the kernel plus every `__device__` helper it calls reads as
/// dead. Fighting the grammar to recover the launch is not worth it —
/// treating `__global__` as a root qualifier fixes the cascade with a
/// substring test, and a kernel genuinely *is* an entry point: the host
/// enters it from outside anything the call graph can see.
fn cuda_qualifiers(text: &str) -> Vec<String> {
    // Only the declaration head, so a `__global__` mentioned in the body
    // (a comment, a string) cannot promote an ordinary function.
    let head = text.split(['{', ';']).next().unwrap_or(text);
    let mut out = Vec::new();
    for q in ["__global__", "__device__", "__host__"] {
        if head
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|t| t == q)
        {
            out.push(q.to_string());
        }
    }
    out
}

#[cfg(test)]
mod cuda_tests {
    use super::cuda_qualifiers;

    #[test]
    fn kernel_qualifier_is_captured_from_the_declaration_head() {
        assert_eq!(
            cuda_qualifiers("__global__ void saxpy(int n, float a) { }"),
            vec!["__global__".to_string()]
        );
        assert!(cuda_qualifiers("void plain(int n) { }").is_empty());
        // A mention inside the body must not promote an ordinary
        // function to an entry point.
        assert!(cuda_qualifiers("void plain() { /* __global__ */ }").is_empty());
        // Substring lookalikes are not qualifiers.
        assert!(cuda_qualifiers("void my__global__helper() {}").is_empty());
    }
}
