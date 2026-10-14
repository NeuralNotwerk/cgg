//! Declared-type normalisation shared by the language plugins and the
//! receiver-type propagator.
//!
//! An annotation that says "a `Foo`, or nothing" — `Optional[Foo]`,
//! `Foo | None`, `Foo?`, `?Foo`, `Foo | undefined`, `Nullable<Foo>`, the
//! string forward reference `"Foo"` — is, for every method called on the
//! value, a `Foo`. Read literally it named the wrapper (`Optional`, booked
//! as a standard-library call) or a string no type is called (`Foo | None`),
//! which typed the receiver *worse* than writing no annotation at all.
//!
//! The language-specific names involved (which wrappers are transparent,
//! which union members are null) live in the [`crate::stdlib`] tables;
//! this module only knows the shapes.

use crate::stdlib::{
    is_maybe_type_wrapper, is_null_type_member, is_union_type_constructor,
};

/// The single type a method call on a value annotated `raw` reaches, in
/// `lang`: nullability markers (`Foo?`, `Foo!`, `?Foo`), null union members
/// (`Foo | None`, `Foo | undefined`, `Union[Foo, None]`), maybe-wrappers
/// (`Optional[Foo]`, `Nullable<Foo>`) and Python's quoted forward
/// references (`"Foo"`) removed. Generic arguments of the result are kept
/// (`Foo<T>?` → `Foo<T>`); callers cut them exactly as they did before.
///
/// `None` when the annotation is empty, only null, or a union of several
/// real types (`Foo | Bar`, `Union[Foo, Bar]`). Picking one member of a
/// real union would be a guess, so such a value is left untyped and gets
/// the reading an unannotated value gets.
pub fn declared_type<'a>(lang: &str, raw: &'a str) -> Option<&'a str> {
    let mut t = raw.trim();
    // Each step removes at least one character, so this terminates; the
    // bound only caps pathological nesting.
    for _ in 0..16 {
        if lang == "python"
            && let Some(inner) = unquote(t)
        {
            t = inner.trim();
            continue;
        }
        if let Some(rest) = t.strip_prefix('?') {
            t = rest.trim_start();
            continue;
        }
        if let Some(rest) = t.strip_suffix('?').or_else(|| t.strip_suffix('!')) {
            t = rest.trim_end();
            continue;
        }
        let members = split_top(t, '|');
        if members.len() > 1 {
            t = single_member(lang, &members)?;
            continue;
        }
        if let Some((head, args)) = subscripted(t) {
            let last = head.rsplit(['.', ':']).next().unwrap_or(head).trim();
            if is_union_type_constructor(lang, last) {
                t = single_member(lang, &split_top(args, ','))?;
                continue;
            }
            if is_maybe_type_wrapper(lang, last) {
                t = split_top(args, ',').first().copied().unwrap_or("").trim();
                continue;
            }
        }
        break;
    }
    (!t.is_empty() && !is_null_type_member(lang, t)).then_some(t)
}

/// The one member of a union that is not null, or `None` when there are
/// none or several.
fn single_member<'a>(lang: &str, members: &[&'a str]) -> Option<&'a str> {
    let mut real = members.iter().map(|m| m.trim()).filter(|m| {
        let bare = if lang == "python" {
            unquote(m).unwrap_or(m).trim()
        } else {
            m
        };
        !bare.is_empty() && !is_null_type_member(lang, bare)
    });
    let one = real.next()?;
    if real.next().is_some() {
        return None;
    }
    Some(one)
}

/// `"Foo"` / `'Foo'` → `Foo`.
fn unquote(t: &str) -> Option<&str> {
    let first = t.chars().next()?;
    if !matches!(first, '"' | '\'') || t.len() < 2 || !t.ends_with(first) {
        return None;
    }
    Some(&t[1..t.len() - 1])
}

/// `Head[args]` / `Head<args>` when the bracket opened after the head
/// closes at the very end.
fn subscripted(t: &str) -> Option<(&str, &str)> {
    let open = t.find(['[', '<'])?;
    let close = matching_close(t, open)?;
    if close + 1 != t.len() || open == 0 {
        return None;
    }
    Some((&t[..open], &t[open + 1..close]))
}

fn matching_close(t: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut prev = '\0';
    for (i, c) in t[open..].char_indices() {
        match c {
            '[' | '<' | '(' | '{' => depth += 1,
            '>' if matches!(prev, '-' | '=') => {}
            ']' | '>' | ')' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
        prev = c;
    }
    None
}

/// `t` cut at every `sep` outside brackets and quotes. An arrow's `>`
/// (`=>`, `->`) closes nothing.
fn split_top(t: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut prev = '\0';
    let mut quote: Option<char> = None;
    for (i, c) in t.char_indices() {
        if let Some(q) = quote {
            if c == q && prev != '\\' {
                quote = None;
            }
            prev = c;
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '[' | '<' | '(' | '{' => depth += 1,
            '>' if matches!(prev, '-' | '=') => {}
            ']' | '>' | ')' | '}' => depth = (depth - 1).max(0),
            c if c == sep && depth == 0 => {
                out.push(&t[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
        prev = c;
    }
    out.push(&t[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::declared_type;

    #[test]
    fn nullable_markers_are_removed() {
        assert_eq!(declared_type("kotlin", "Foo?"), Some("Foo"));
        assert_eq!(declared_type("swift", "Foo!"), Some("Foo"));
        assert_eq!(declared_type("swift", "Foo?"), Some("Foo"));
        assert_eq!(declared_type("csharp", "Foo?"), Some("Foo"));
        assert_eq!(declared_type("dart", "Foo<T>?"), Some("Foo<T>"));
        assert_eq!(declared_type("php", "?Foo"), Some("Foo"));
        assert_eq!(declared_type("kotlin", "Foo"), Some("Foo"));
    }

    #[test]
    fn a_union_with_one_real_member_is_that_member() {
        assert_eq!(declared_type("python", "Foo | None"), Some("Foo"));
        assert_eq!(declared_type("python", "None | Foo"), Some("Foo"));
        assert_eq!(declared_type("typescript", "Foo | undefined"), Some("Foo"));
        assert_eq!(
            declared_type("typescript", "Foo | null | undefined"),
            Some("Foo")
        );
        assert_eq!(declared_type("php", "Foo|null"), Some("Foo"));
        assert_eq!(declared_type("php", "null|Foo"), Some("Foo"));
        assert_eq!(declared_type("scala", "Foo | Null"), Some("Foo"));
        assert_eq!(declared_type("python", "Union[Foo, None]"), Some("Foo"));
        assert_eq!(
            declared_type("python", "typing.Union[None, Foo]"),
            Some("Foo")
        );
    }

    #[test]
    fn a_union_of_several_real_types_is_untyped() {
        assert_eq!(declared_type("python", "Foo | Bar"), None);
        assert_eq!(declared_type("python", "Union[Foo, Bar]"), None);
        assert_eq!(declared_type("python", "Optional[Union[Foo, Bar]]"), None);
        assert_eq!(declared_type("typescript", "Foo | Bar | null"), None);
        assert_eq!(declared_type("php", "Foo|Bar"), None);
        assert_eq!(declared_type("python", "None"), None);
        // Null in a language with no null members is a real member.
        assert_eq!(declared_type("kotlin", "Foo | None"), None);
    }

    #[test]
    fn maybe_wrappers_are_their_argument_and_containers_are_not() {
        assert_eq!(declared_type("python", "Optional[Foo]"), Some("Foo"));
        assert_eq!(declared_type("python", "typing.Optional[Foo]"), Some("Foo"));
        assert_eq!(
            declared_type("python", "ClassVar[Optional[Foo]]"),
            Some("Foo")
        );
        assert_eq!(
            declared_type("python", "Annotated[Foo, Depends(x)]"),
            Some("Foo")
        );
        assert_eq!(declared_type("csharp", "Nullable<Pt>"), Some("Pt"));
        assert_eq!(declared_type("csharp", "System.Nullable<Pt>"), Some("Pt"));
        assert_eq!(declared_type("swift", "Optional<Foo>"), Some("Foo"));
        // Containers keep their own type.
        assert_eq!(declared_type("python", "list[Foo]"), Some("list[Foo]"));
        assert_eq!(
            declared_type("python", "Optional[list[Foo]]"),
            Some("list[Foo]")
        );
        assert_eq!(declared_type("rust", "Option<Foo>"), Some("Option<Foo>"));
        assert_eq!(
            declared_type("java", "Optional<Foo>"),
            Some("Optional<Foo>")
        );
        assert_eq!(declared_type("scala", "Option[Foo]"), Some("Option[Foo]"));
        // A wrapper of another language is not transparent here.
        assert_eq!(
            declared_type("kotlin", "Optional<Foo>"),
            Some("Optional<Foo>")
        );
    }

    #[test]
    fn python_forward_references_name_the_class() {
        assert_eq!(declared_type("python", "\"Foo\""), Some("Foo"));
        assert_eq!(declared_type("python", "'Foo'"), Some("Foo"));
        assert_eq!(declared_type("python", "\"Optional[Foo]\""), Some("Foo"));
        assert_eq!(declared_type("python", "Optional['Foo']"), Some("Foo"));
        assert_eq!(declared_type("python", "\"Foo\" | None"), Some("Foo"));
        // Only Python writes a type as a string.
        assert_eq!(declared_type("typescript", "\"a\""), Some("\"a\""));
    }

    #[test]
    fn arrows_and_nested_brackets_do_not_split_a_member() {
        assert_eq!(
            declared_type("typescript", "((x: A) => B) | null"),
            Some("((x: A) => B)")
        );
        assert_eq!(
            declared_type("python", "Optional[Callable[[A], B]]"),
            Some("Callable[[A], B]")
        );
    }
}
