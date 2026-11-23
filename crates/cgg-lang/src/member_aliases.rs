//! Member-read aliases: a local bound to a member read
//! (`const f = obj.load`, `f := x.Load`, `let f = obj.load`,
//! `final f = obj.load`), so a bare call through the local (`f()`) is
//! the member call it names.
//!
//! The Python and Lua plugins record these themselves; this walk covers
//! the languages whose plugins do not (`ALIAS_LANGS`), across their
//! grammars' declaration shapes. The declaration byte is taken from the
//! file's own scope table — the binding the locals walk recorded for the
//! name — so `scope::FileLocals` matches the alias to exactly the
//! binding in scope at a call, however the language places it.

use cgg_core::{LocalBinding, MemberAlias};
use tree_sitter::{Node, Tree};

/// Languages this walk records aliases for.
pub const ALIAS_LANGS: &[&str] = &["javascript", "typescript", "go", "swift", "dart"];

/// `(local name node, member read node)` of a declaration, when its
/// value is a member read.
fn alias_parts<'t>(lang: &str, n: Node<'t>) -> Option<(Node<'t>, Node<'t>)> {
    match (lang, n.kind()) {
        ("javascript" | "typescript", "variable_declarator") => {
            let name = n.child_by_field_name("name")?;
            let value = n.child_by_field_name("value")?;
            (name.kind() == "identifier" && value.kind() == "member_expression")
                .then_some((name, value))
        }
        ("dart", "initialized_variable_definition") => {
            let name = n.child_by_field_name("name")?;
            let value = n.child_by_field_name("value")?;
            (name.kind() == "identifier" && value.kind() == "member_expression")
                .then_some((name, value))
        }
        ("go", "short_var_declaration") => {
            let left = n.child_by_field_name("left")?;
            let right = n.child_by_field_name("right")?;
            (left.named_child_count() == 1 && right.named_child_count() == 1)
                .then_some(())?;
            let name = left.named_child(0)?;
            let value = right.named_child(0)?;
            (name.kind() == "identifier" && value.kind() == "selector_expression")
                .then_some((name, value))
        }
        ("go", "var_spec") => {
            let name = n.child_by_field_name("name")?;
            let value = n.child_by_field_name("value")?;
            (value.named_child_count() == 1).then_some(())?;
            let value = value.named_child(0)?;
            (value.kind() == "selector_expression").then_some((name, value))
        }
        ("swift", "property_declaration") => {
            let pattern = n.child_by_field_name("name")?;
            let name = pattern.child_by_field_name("bound_identifier")?;
            let value = n.child_by_field_name("value")?;
            (value.kind() == "navigation_expression").then_some((name, value))
        }
        _ => None,
    }
}

/// `(head, member)` of a member read: `obj` and `load` in `obj.load`,
/// `this.svc` and `load` in `this.svc.load`.
fn head_member<'s>(lang: &str, value: Node, src: &'s [u8]) -> Option<(&'s str, &'s str)> {
    let text = |n: Node| std::str::from_utf8(&src[n.byte_range()]).ok();
    let (head, member) = match lang {
        "go" => (
            value.child_by_field_name("operand")?,
            value.child_by_field_name("field")?,
        ),
        "swift" => {
            let target = value.child_by_field_name("target")?;
            let suffix = value.child_by_field_name("suffix")?;
            (target, suffix.child_by_field_name("suffix")?)
        }
        _ => (
            value.child_by_field_name("object")?,
            value.child_by_field_name("property")?,
        ),
    };
    let (h, m) = (text(head)?.trim(), text(member)?.trim());
    let ident = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    };
    (ident(m) && !h.is_empty() && h.len() < 200 && !h.contains(['(', '\n', '[', '?']))
        .then_some((h, m))
}

/// The member-read aliases of a file of `lang`, given its local bindings.
pub fn collect(
    lang: &str,
    tree: &Tree,
    src: &[u8],
    locals: &[LocalBinding],
) -> Vec<MemberAlias> {
    if !ALIAS_LANGS.contains(&lang) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    let mut done = false;
    while !done {
        let n = cursor.node();
        if !cursor.goto_first_child() {
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    done = true;
                    break;
                }
            }
        }
        let Some((name_node, value)) = alias_parts(lang, n) else {
            continue;
        };
        let Ok(name) = std::str::from_utf8(&src[name_node.byte_range()]) else {
            continue;
        };
        let Some((head, member)) = head_member(lang, value, src) else {
            continue;
        };
        // The same-named read (`const load = obj.load`) is no alias: the
        // name the call writes is the member's own.
        if name == member || name.is_empty() {
            continue;
        }
        // The binding this declaration made: the latest declaration of
        // the name at or before it, in scope after the value.
        let at = name_node.start_byte() as u32;
        let Some(binding) = locals
            .iter()
            .filter(|b| {
                !b.param
                    && b.name == name
                    && b.decl_byte <= at
                    && b.start_byte >= value.end_byte() as u32
            })
            .max_by_key(|b| b.decl_byte)
        else {
            continue;
        };
        out.push(MemberAlias {
            name: name.to_string(),
            head: head.to_string(),
            member: member.to_string(),
            decl_byte: binding.decl_byte,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases(lang: &str, src: &str) -> Vec<(String, String, String)> {
        let reg = crate::PluginRegistry::with_v1_plugins();
        let p = reg.all().iter().find(|p| p.id() == lang).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&p.ts_language()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let locals = crate::locals::collect(lang, &tree, src.as_bytes());
        collect(lang, &tree, src.as_bytes(), &locals)
            .into_iter()
            .map(|a| (a.name, a.head, a.member))
            .collect()
    }

    fn t(a: &str, b: &str, c: &str) -> (String, String, String) {
        (a.into(), b.into(), c.into())
    }

    #[test]
    fn member_reads_bound_to_a_local_are_aliases_in_every_language() {
        assert_eq!(
            aliases("javascript", "function g(){ const f = obj.load; f(); }"),
            vec![t("f", "obj", "load")]
        );
        assert_eq!(
            aliases("typescript", "function g(){ let f = this.svc.load; f(); }"),
            vec![t("f", "this.svc", "load")]
        );
        assert_eq!(
            aliases(
                "go",
                "package p\nfunc g(){ f := x.Load; f(); var h = y.Save; h() }"
            ),
            vec![t("f", "x", "Load"), t("h", "y", "Save")]
        );
        assert_eq!(
            aliases("swift", "func g(){ let f = obj.load; f() }"),
            vec![t("f", "obj", "load")]
        );
        assert_eq!(
            aliases("dart", "void g(){ final f = a.b.save; f(); }"),
            vec![t("f", "a.b", "save")]
        );
    }

    #[test]
    fn same_named_reads_and_calls_are_not_aliases() {
        assert!(
            aliases(
                "javascript",
                "function g(){ const load = obj.load; const x = obj.make(); }"
            )
            .is_empty()
        );
        assert!(aliases("go", "package p\nfunc g(){ v := x.Load() }").is_empty());
    }
}
