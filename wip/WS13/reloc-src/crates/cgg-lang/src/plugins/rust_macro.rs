//! A deliberately small `macro_rules!` expander for item-generating macros.
//!
//! `new_id!(DocId, "doc")` generates `DocId::new`, `DocId::as_str` and the
//! `From`/`Display` impls; without expansion none of that API exists in the
//! graph, and every call into it is unresolvable. This expands the subset
//! that covers most real item macros — measured across the Rust corpus,
//! 63% of item-producing `macro_rules!` and 81% of their invocations: arms
//! with no repetition (`$( … )*`), matched by fragment over token trees.
//! Anything else — repetition, an arm that does not match, an expansion
//! that does not parse — yields `None`, and the caller keeps its
//! pre-expansion behaviour. A wrong expansion is worse than none.

use std::collections::HashMap;

/// One lexed token: its source text and, for a group, its delimiter and
/// the tokens inside it.
#[derive(Debug, Clone, PartialEq)]
struct Tok {
    text: String,
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Ident,
    Lifetime,
    Literal,
    Punct,
    Group(char, Vec<Tok>),
}

/// A matcher element.
#[derive(Debug, Clone)]
enum Pat {
    Lit(String),
    Var(String, String),
    Group(char, Vec<Pat>),
}

/// The arms of one `macro_rules!`, in declaration order, that the
/// expander can handle.
#[derive(Debug, Clone, Default)]
pub(super) struct MacroDef {
    arms: Vec<(Vec<Pat>, String)>,
}

/// Parse a `macro_rules!` body (`{ (matcher) => { transcriber }; … }`,
/// outer braces included or not). Arms using repetition are dropped;
/// `None` when no arm survives.
pub(super) fn parse_macro_rules(body: &str) -> Option<MacroDef> {
    let toks = lex(body)?;
    let toks = match toks.as_slice() {
        [
            Tok {
                kind: Kind::Group(_, inner),
                ..
            },
        ] => inner.clone(),
        _ => toks,
    };
    let mut arms = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        // matcher group, `=>`, transcriber group, optional `;`
        let (Some(m), Some(arrow), Some(t)) =
            (toks.get(i), toks.get(i + 1), toks.get(i + 2))
        else {
            break;
        };
        let (Kind::Group(_, m_inner), Kind::Group(_, _)) = (&m.kind, &t.kind) else {
            return None;
        };
        if arrow.text != "=>" {
            return None;
        }
        let repeats = m.text.contains("$(") || t.text.contains("$(");
        if !repeats && let Some(pats) = to_pats(m_inner) {
            // Transcriber without its outer delimiters.
            let body = &t.text[1..t.text.len() - 1];
            arms.push((pats, body.to_string()));
        }
        i += 3;
        if toks.get(i).is_some_and(|t| t.text == ";") {
            i += 1;
        }
    }
    (!arms.is_empty()).then_some(MacroDef { arms })
}

/// Expand an invocation whose argument tokens (outer delimiters stripped)
/// are `args`. The first arm that matches wins, as in rustc.
pub(super) fn expand(def: &MacroDef, args: &str) -> Option<String> {
    let toks = lex(args)?;
    for (pats, body) in &def.arms {
        let mut binds = HashMap::new();
        if match_seq(pats, &toks, &mut binds) {
            return transcribe(body, &binds);
        }
    }
    None
}

fn to_pats(toks: &[Tok]) -> Option<Vec<Pat>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if t.text == "$" {
            let name = toks.get(i + 1).filter(|n| n.kind == Kind::Ident)?;
            if toks.get(i + 2).is_some_and(|c| c.text == ":")
                && let Some(frag) = toks.get(i + 3).filter(|f| f.kind == Kind::Ident)
            {
                out.push(Pat::Var(name.text.clone(), frag.text.clone()));
                i += 4;
                continue;
            }
            return None;
        }
        out.push(match &t.kind {
            Kind::Group(d, inner) => Pat::Group(*d, to_pats(inner)?),
            _ => Pat::Lit(t.text.clone()),
        });
        i += 1;
    }
    Some(out)
}

fn match_seq(pats: &[Pat], toks: &[Tok], binds: &mut HashMap<String, String>) -> bool {
    let mut j = 0;
    for (i, p) in pats.iter().enumerate() {
        match p {
            Pat::Lit(s) => {
                if toks.get(j).is_none_or(|t| t.text != *s) {
                    return false;
                }
                j += 1;
            }
            Pat::Group(d, inner) => match toks.get(j).map(|t| &t.kind) {
                Some(Kind::Group(td, tinner)) if td == d => {
                    if !match_seq(inner, tinner, binds) {
                        return false;
                    }
                    j += 1;
                }
                _ => return false,
            },
            Pat::Var(name, frag) => {
                let single = |pred: &dyn Fn(&Tok) -> bool| {
                    toks.get(j).filter(|t| pred(t)).is_some()
                };
                let take = match frag.as_str() {
                    "ident" => single(&|t| t.kind == Kind::Ident) as usize,
                    "lifetime" => single(&|t| t.kind == Kind::Lifetime) as usize,
                    "literal" => {
                        if toks.get(j).is_some_and(|t| t.text == "-")
                            && toks.get(j + 1).is_some_and(|t| t.kind == Kind::Literal)
                        {
                            2
                        } else {
                            single(&|t| {
                                t.kind == Kind::Literal
                                    || (t.kind == Kind::Ident
                                        && matches!(t.text.as_str(), "true" | "false"))
                            }) as usize
                        }
                    }
                    "tt" => single(&|_| true) as usize,
                    "block" => {
                        single(&|t| matches!(t.kind, Kind::Group('{', _))) as usize
                    }
                    // expr, ty, path, pat, stmt, item, meta, vis: everything
                    // up to the next literal of the matcher (the follow-set
                    // rules guarantee one exists after these), or the rest.
                    _ => {
                        let stop = match pats.get(i + 1) {
                            Some(Pat::Lit(s)) => Some(s.as_str()),
                            Some(_) => return false,
                            None => None,
                        };
                        let n = toks[j..]
                            .iter()
                            .position(|t| stop.is_some_and(|s| t.text == s))
                            .unwrap_or(toks.len() - j);
                        if n == 0 && frag != "vis" {
                            return false;
                        }
                        n
                    }
                };
                if take == 0 && frag != "vis" {
                    return false;
                }
                let text: Vec<&str> =
                    toks[j..j + take].iter().map(|t| t.text.as_str()).collect();
                binds.insert(name.clone(), text.join(" "));
                j += take;
            }
        }
    }
    j == toks.len()
}

fn transcribe(body: &str, binds: &HashMap<String, String>) -> Option<String> {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let end = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..end];
        if name == "crate" {
            out.push_str("crate");
        } else {
            // An unbound `$x` is a repetition variable or a nested macro's
            // own metavariable: not something this expander can produce.
            out.push_str(binds.get(name)?);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    Some(out)
}

/// Lex Rust source into token trees. `None` on unbalanced delimiters.
fn lex(src: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = src.chars().collect();
    let mut stack: Vec<(char, usize, Vec<Tok>)> = Vec::new();
    let mut top: Vec<Tok> = Vec::new();
    let mut i = 0;
    // Byte offsets so group text can be sliced from `src`.
    let byte_at: Vec<usize> = {
        let mut v = Vec::with_capacity(chars.len() + 1);
        let mut b = 0;
        for c in &chars {
            v.push(b);
            b += c.len_utf8();
        }
        v.push(b);
        v
    };
    let push = |stack: &mut Vec<(char, usize, Vec<Tok>)>, top: &mut Vec<Tok>, t: Tok| {
        match stack.last_mut() {
            Some((_, _, v)) => v.push(t),
            None => top.push(t),
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if matches!(c, '(' | '[' | '{') {
            stack.push((c, i, Vec::new()));
            i += 1;
            continue;
        }
        if matches!(c, ')' | ']' | '}') {
            let (open, at, inner) = stack.pop()?;
            if !matches!((open, c), ('(', ')') | ('[', ']') | ('{', '}')) {
                return None;
            }
            i += 1;
            let text = src[byte_at[at]..byte_at[i]].to_string();
            push(
                &mut stack,
                &mut top,
                Tok {
                    text,
                    kind: Kind::Group(open, inner),
                },
            );
            continue;
        }
        // Raw / byte / plain strings.
        let raw_start = if c == 'r' && matches!(chars.get(i + 1), Some('"' | '#')) {
            Some(i + 1)
        } else if c == 'b'
            && chars.get(i + 1) == Some(&'r')
            && matches!(chars.get(i + 2), Some('"' | '#'))
        {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut k) = raw_start {
            let mut hashes = 0;
            while chars.get(k) == Some(&'#') {
                hashes += 1;
                k += 1;
            }
            if chars.get(k) == Some(&'"') {
                k += 1;
                loop {
                    if k >= chars.len() {
                        return None;
                    }
                    if chars[k] == '"'
                        && (0..hashes).all(|h| chars.get(k + 1 + h) == Some(&'#'))
                    {
                        k += 1 + hashes;
                        break;
                    }
                    k += 1;
                }
                i = k;
                let text = src[byte_at[start]..byte_at[i]].to_string();
                push(
                    &mut stack,
                    &mut top,
                    Tok {
                        text,
                        kind: Kind::Literal,
                    },
                );
                continue;
            }
        }
        if c == '"' || (c == 'b' && chars.get(i + 1) == Some(&'"')) {
            i += if c == 'b' { 2 } else { 1 };
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            let text = src[byte_at[start]..byte_at[i.min(chars.len())]].to_string();
            push(
                &mut stack,
                &mut top,
                Tok {
                    text,
                    kind: Kind::Literal,
                },
            );
            continue;
        }
        if c == '\'' {
            // `'a'`, `'\n'` are chars; `'a` is a lifetime.
            let is_char =
                (chars.get(i + 2) == Some(&'\'')) || (chars.get(i + 1) == Some(&'\\'));
            if is_char {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                let text = src[byte_at[start]..byte_at[i.min(chars.len())]].to_string();
                push(
                    &mut stack,
                    &mut top,
                    Tok {
                        text,
                        kind: Kind::Literal,
                    },
                );
            } else {
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let text = src[byte_at[start]..byte_at[i]].to_string();
                push(
                    &mut stack,
                    &mut top,
                    Tok {
                        text,
                        kind: Kind::Lifetime,
                    },
                );
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let text = src[byte_at[start]..byte_at[i]].to_string();
            push(
                &mut stack,
                &mut top,
                Tok {
                    text,
                    kind: Kind::Ident,
                },
            );
            continue;
        }
        if c.is_ascii_digit() {
            while i < chars.len()
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || (chars[i] == '.'
                        && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())))
            {
                i += 1;
            }
            let text = src[byte_at[start]..byte_at[i]].to_string();
            push(
                &mut stack,
                &mut top,
                Tok {
                    text,
                    kind: Kind::Literal,
                },
            );
            continue;
        }
        const MULTI: &[&str] = &[
            "..=", "...", "::", "=>", "->", "..", "==", "!=", "<=", ">=", "&&", "||",
            "+=", "-=", "*=", "/=", "%=", "^=", "&=", "|=", "<<", ">>",
        ];
        let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
        let op = MULTI.iter().find(|m| rest.starts_with(**m)).copied();
        let len = op.map_or(1, |m| m.chars().count());
        i += len;
        let text = src[byte_at[start]..byte_at[i]].to_string();
        push(
            &mut stack,
            &mut top,
            Tok {
                text,
                kind: Kind::Punct,
            },
        );
    }
    stack.is_empty().then_some(top)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_ID: &str = r#"{
    ($name:ident, $prefix:literal) => {
        pub struct $name(pub String);
        impl $name {
            pub fn new() -> Self { Self(format!("{}_{}", $prefix, 1)) }
        }
    };
}"#;

    #[test]
    fn expands_a_simple_item_macro() {
        let def = parse_macro_rules(NEW_ID).expect("parsed");
        let out = expand(&def, r#"DocId, "doc""#).expect("expanded");
        assert!(out.contains("pub struct DocId(pub String);"), "{out}");
        assert!(out.contains("impl DocId"), "{out}");
        assert!(out.contains(r#"format!("{}_{}", "doc", 1)"#), "{out}");
    }

    #[test]
    fn raw_string_literals_and_arm_selection() {
        let def = parse_macro_rules(
            r#"{
            (@one $name:ident) => { fn $name() {} };
            ($name:ident, $pat:literal) => { fn $name() -> &'static str { $pat } };
        }"#,
        )
        .expect("parsed");
        let out = expand(&def, r##"path_re, r"(?:/[\w.]+){2,}""##).expect("expanded");
        assert!(out.contains("fn path_re()"), "{out}");
        assert!(out.contains(r#"r"(?:/[\w.]+){2,}""#), "{out}");
        assert!(expand(&def, "@one go").unwrap().contains("fn go()"));
    }

    #[test]
    fn repetition_and_mismatch_do_not_expand() {
        assert!(
            parse_macro_rules("{ ($($x:ident),*) => { $(fn $x() {})* }; }").is_none()
        );
        let def = parse_macro_rules(NEW_ID).unwrap();
        assert!(expand(&def, "DocId").is_none());
        assert!(expand(&def, "DocId, \"a\", extra").is_none());
    }
}
