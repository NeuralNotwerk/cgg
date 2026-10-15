//! Event bindings written in markup next to a C# file.
//!
//! A WPF/Avalonia window wires `Click="OnSave"` in `Main.xaml`, a Blazor
//! component `@onclick="Save"` in `Counter.razor`, a Godot scene
//! `[connection signal="timeout" … method="OnTimeout"]` in `Main.tscn`.
//! The handler lives in the code-behind (`Main.xaml.cs`, `Counter.razor.cs`,
//! `Main.cs`) and nothing in C# calls it. This module reads the sibling
//! markup file of a code-behind and returns the bindings that name a
//! method the code-behind defines, so framework detection can mint the
//! UI entry. It is a structural reader, not a parser: it looks only at
//! `name="value"` attribute pairs and scene `[connection …]` lines, and
//! never produces an edge.

use std::path::{Path, PathBuf};

use cgg_core::MarkupBinding;

/// Markup files larger than this are not read (generated or vendored).
const MAX_MARKUP_BYTES: u64 = 2 * 1024 * 1024;

/// A method the code-behind defines, as the reader needs it.
pub(crate) struct Handler<'a> {
    pub name: &'a str,
    /// The method's signature as written, for the XAML event-handler shape.
    pub signature: &'a str,
}

/// The sibling markup of `path` and its kind, if it has one.
fn sibling(path: &Path) -> Option<(PathBuf, &'static str)> {
    let name = path.file_name()?.to_str()?;
    for (suffix, kind) in [
        (".xaml.cs", "xaml"),
        (".axaml.cs", "xaml"),
        (".razor.cs", "razor"),
    ] {
        if name.ends_with(suffix) {
            let markup = path.with_file_name(&name[..name.len() - 3]);
            return markup.is_file().then_some((markup, kind));
        }
    }
    // Godot names a scene and its script alike: `Main.tscn` + `Main.cs`.
    let stem = name.strip_suffix(".cs")?;
    let scene = path.with_file_name(format!("{stem}.tscn"));
    scene.is_file().then_some((scene, "tscn"))
}

/// Bindings in the sibling markup of `path` whose handler is one of
/// `handlers`. Empty when there is no sibling or it cannot be read.
pub(crate) fn bindings(path: &Path, handlers: &[Handler<'_>]) -> Vec<MarkupBinding> {
    if handlers.is_empty() {
        return Vec::new();
    }
    let Some((markup, kind)) = sibling(path) else {
        return Vec::new();
    };
    if std::fs::metadata(&markup).map_or(true, |m| m.len() > MAX_MARKUP_BYTES) {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(&markup) else {
        return Vec::new();
    };
    read(&text, kind, handlers)
}

/// The reader proper, separated from file access for testing.
pub(crate) fn read(
    text: &str,
    kind: &str,
    handlers: &[Handler<'_>],
) -> Vec<MarkupBinding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line_no = i as u32 + 1;
        if kind == "tscn" {
            let l = line.trim_start();
            if !l.starts_with("[connection") {
                continue;
            }
            let pairs = pairs(l);
            let get = |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
            if let (Some(signal), Some(method)) = (get("signal"), get("method"))
                && handlers.iter().any(|h| h.name == method)
            {
                out.push(MarkupBinding {
                    kind: kind.into(),
                    event: signal.into(),
                    handler: method.into(),
                    line: line_no,
                });
            }
            continue;
        }
        for (attr, value) in pairs(line) {
            let value = if kind == "razor" {
                value.strip_prefix('@').unwrap_or(value)
            } else {
                value
            };
            if !is_identifier(value) {
                continue;
            }
            let Some(h) = handlers.iter().find(|h| h.name == value) else {
                continue;
            };
            let event_attr = match kind {
                // Blazor: DOM events are `@on…`; a component's
                // `EventCallback` parameters are `On…` by convention.
                "razor" => {
                    attr.strip_prefix("@on").is_some_and(|r| !r.is_empty())
                        || attr.strip_prefix("On").is_some_and(|r| {
                            r.starts_with(|c: char| c.is_ascii_uppercase())
                        })
                }
                // XAML: any attribute can name a method, but only an event
                // handler has the `(sender, …EventArgs)` shape, which keeps
                // `Text="Save"` from binding a `Save()` method.
                _ => !attr.contains(':') && is_event_handler_signature(h.signature),
            };
            if event_attr {
                out.push(MarkupBinding {
                    kind: kind.into(),
                    event: attr.into(),
                    handler: value.into(),
                    line: line_no,
                });
            }
        }
    }
    out
}

/// `(object sender, RoutedEventArgs e)`: two parameters, the second an
/// `…EventArgs`/`…Args` type.
fn is_event_handler_signature(sig: &str) -> bool {
    let Some(open) = sig.find('(') else {
        return false;
    };
    let Some(close) = sig.rfind(')') else {
        return false;
    };
    if close <= open {
        return false;
    }
    let params: Vec<&str> = sig[open + 1..close].split(',').map(str::trim).collect();
    if params.len() != 2 {
        return false;
    }
    let ty = params[1]
        .split_whitespace()
        .rev()
        .nth(1)
        .unwrap_or("")
        .trim_end_matches('?');
    ty.ends_with("Args")
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Every `name="value"` pair on one line, in order.
fn pairs(line: &str) -> Vec<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = line[i..].find("=\"") {
        let eq = i + off;
        let mut start = eq;
        while start > 0 {
            let c = bytes[start - 1];
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'@' | b'-')
            {
                start -= 1;
            } else {
                break;
            }
        }
        let vstart = eq + 2;
        let Some(vlen) = line[vstart..].find('"') else {
            break;
        };
        if start < eq {
            out.push((&line[start..eq], &line[vstart..vstart + vlen]));
        }
        i = vstart + vlen + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h<'a>(name: &'a str, signature: &'a str) -> Handler<'a> {
        Handler { name, signature }
    }

    #[test]
    fn xaml_binds_only_event_handler_shaped_methods() {
        let hs = [
            h("OnSave", "void OnSave(object sender, RoutedEventArgs e)"),
            h("Save", "void Save()"),
        ];
        let b = read(
            r#"<Button Click="OnSave" Text="Save" x:Name="OnSave" Content="{Binding X}"/>"#,
            "xaml",
            &hs,
        );
        assert_eq!(b.len(), 1, "{b:?}");
        assert_eq!(
            (b[0].event.as_str(), b[0].handler.as_str()),
            ("Click", "OnSave")
        );
    }

    #[test]
    fn razor_binds_dom_and_component_events() {
        let hs = [
            h("Inc", "void Inc()"),
            h("Pick", "void Pick()"),
            h("Title", "string Title()"),
        ];
        let b = read(
            r#"<button @onclick="Inc"></button><Grid OnRowClick="@Pick" Title="Title" />"#,
            "razor",
            &hs,
        );
        let got: Vec<_> = b
            .iter()
            .map(|b| (b.event.as_str(), b.handler.as_str()))
            .collect();
        assert_eq!(got, vec![("@onclick", "Inc"), ("OnRowClick", "Pick")]);
    }

    #[test]
    fn godot_scene_connections_name_the_signal() {
        let hs = [h("OnMobTimerTimeout", "void OnMobTimerTimeout()")];
        let b = read(
            "[connection signal=\"timeout\" from=\"MobTimer\" to=\".\" method=\"OnMobTimerTimeout\"]\n\
             [connection signal=\"x\" from=\"A\" to=\".\" method=\"Missing\"]",
            "tscn",
            &hs,
        );
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].event.as_str(), b[0].line), ("timeout", 1));
    }
}
