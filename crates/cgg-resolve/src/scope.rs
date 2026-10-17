//! Value bindings in scope at a call site (WS14, FINDINGS A1/A4
//! generalisation).
//!
//! The plugins record every parameter and local variable with the span it
//! is in scope over (`FileFacts::locals`, see `cgg_lang::locals`). A bare
//! call through such a name calls the value it holds: neither a same-named
//! function in another file nor one declared further out in this file is
//! the callee. A receiver headed by one (`c.json()` with a parameter `c`)
//! is that value, not an import of the same name.

use std::collections::HashMap;

use cgg_core::{FileFacts, RefRecord};

/// A binding's span, the index of its innermost enclosing span of the
/// same name, where its declaration starts, and whether it is a
/// parameter ([`cgg_core::LocalBinding::param`]).
type Span = (u32, u32, Option<usize>, u32, bool);

/// The binding of a name in scope at a site (see
/// [`FileLocals::binding_decl_at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) start: u32,
    pub(crate) end: u32,
    /// Where its declaration starts.
    pub(crate) decl: u32,
    /// A parameter (of a function, lambda or closure), not a local.
    pub(crate) param: bool,
}

/// One file's value bindings, by name.
#[derive(Debug, Default)]
pub(crate) struct FileLocals {
    /// Names the file's imports bind: a bare call through a local of that
    /// name is left to the import reading (a component's prop that is the
    /// bound action creator it imports).
    imported: std::collections::HashSet<String>,
    /// Per name: binding spans sorted by start (outermost first at equal
    /// starts), each with the index of its innermost enclosing span of
    /// the same name — a minified bundle binds one short name thousands
    /// of times, so a lookup climbs this chain instead of scanning.
    spans: HashMap<String, Vec<Span>>,
}

impl FileLocals {
    pub(crate) fn build(facts: &FileFacts) -> Self {
        // An import the declaration itself makes (`const m =
        // require('./m')` inside a function) is that import's binding,
        // which the module resolver follows: not a shadow of it.
        let mut import_sites: HashMap<&str, Vec<u32>> = HashMap::new();
        for imp in &facts.imports {
            for item in imp.alias.split(',') {
                let item = item.trim();
                let local = item
                    .rsplit_once(" as ")
                    .map_or(item, |(_, a)| a.trim())
                    .trim();
                if !local.is_empty() {
                    import_sites.entry(local).or_default().push(imp.site_byte);
                }
            }
        }
        let mut raw: HashMap<String, Vec<(u32, u32, u32, bool)>> = HashMap::new();
        for l in &facts.locals {
            let is_the_import = import_sites.get(l.name.as_str()).is_some_and(|sites| {
                sites.iter().any(|b| l.decl_byte <= *b && *b < l.start_byte)
            });
            if is_the_import {
                continue;
            }
            raw.entry(l.name.clone()).or_default().push((
                l.start_byte,
                l.end_byte,
                l.decl_byte,
                l.param,
            ));
        }
        let mut spans: HashMap<String, Vec<Span>> = HashMap::new();
        for (name, mut v) in raw {
            v.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then(b.1.cmp(&a.1))
                    .then(a.2.cmp(&b.2))
                    .then(a.3.cmp(&b.3))
            });
            v.dedup();
            let mut out: Vec<Span> = Vec::with_capacity(v.len());
            let mut open: Vec<usize> = Vec::new();
            for (s, e, d, p) in v {
                while open
                    .last()
                    .is_some_and(|&o| !(out[o].0 <= s && e <= out[o].1))
                {
                    open.pop();
                }
                out.push((s, e, open.last().copied(), d, p));
                open.push(out.len() - 1);
            }
            spans.insert(name, out);
        }
        let imported = import_sites.keys().map(|k| (*k).to_string()).collect();
        FileLocals { spans, imported }
    }

    /// The innermost binding of `name` in scope at `site`, as its span.
    pub(crate) fn binding_at(&self, name: &str, site: u32) -> Option<(u32, u32)> {
        self.binding_decl_at(name, site).map(|b| (b.start, b.end))
    }

    /// [`Self::binding_at`] with where the binding's declaration starts.
    pub(crate) fn binding_decl_at(&self, name: &str, site: u32) -> Option<Binding> {
        let v = self.spans.get(name)?;
        let pos = v.partition_point(|(s, _, _, _, _)| *s <= site);
        let mut cur = pos.checked_sub(1);
        while let Some(i) = cur {
            let (s, e, parent, d, p) = v[i];
            if s <= site && site < e {
                return Some(Binding {
                    start: s,
                    end: e,
                    decl: d,
                    param: p,
                });
            }
            cur = parent;
        }
        None
    }

    /// Whether `name` is a value binding in scope at `site`.
    pub(crate) fn binds(&self, name: &str, site: u32) -> bool {
        self.binding_at(name, site).is_some()
    }

    /// A bare call (`f()`, no receiver and no dot) through a value
    /// binding, which no other file's function can be the callee of.
    pub(crate) fn shadows_bare_call(&self, r: &RefRecord) -> bool {
        r.receiver_hint.is_empty()
            && !r.dot_call
            && !r.macro_call
            && !self.imported.contains(r.name.as_str())
            && self.binds(&r.name, r.site_byte)
    }

    /// Whether the leading identifier of the call's receiver is a value
    /// binding at the site — then it is not a module or import of that
    /// name.
    pub(crate) fn shadows_receiver_head(&self, r: &RefRecord) -> bool {
        let rh = r.receiver_hint.trim();
        let rh = rh.strip_prefix("new ").unwrap_or(rh);
        let end = rh
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
            .map_or(rh.len(), |(i, _)| i);
        let head = &rh[..end];
        !head.is_empty() && self.binds(head, r.site_byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::{LocalBinding, ids::FileId};

    fn facts() -> FileFacts {
        let mut f = FileFacts::new(FileId::new(0), "a.js".into(), "javascript");
        f.locals = vec![
            LocalBinding {
                name: "cb".into(),
                start_byte: 10,
                end_byte: 50,
                decl_byte: 10,
                param: false,
            },
            LocalBinding {
                name: "c".into(),
                start_byte: 10,
                end_byte: 50,
                decl_byte: 10,
                param: false,
            },
        ];
        f
    }

    fn call(name: &str, recv: &str, at: u32) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_byte: at,
            ..Default::default()
        }
    }

    #[test]
    fn a_bare_call_through_a_binding_in_scope_is_shadowed() {
        let l = FileLocals::build(&facts());
        assert!(l.shadows_bare_call(&call("cb", "", 20)));
        // Out of the binding's scope, or with a receiver: not shadowed.
        assert!(!l.shadows_bare_call(&call("cb", "", 60)));
        assert!(!l.shadows_bare_call(&call("cb", "x", 20)));
        assert!(l.shadows_receiver_head(&call("json", "c", 20)));
        assert!(l.shadows_receiver_head(&call("json", "c.res", 20)));
        assert!(!l.shadows_receiver_head(&call("json", "cx", 20)));
    }

    /// `Err(cb) => cb!(…)`: a macro is not a value, so a binding of the
    /// same name does not shadow it.
    #[test]
    fn a_macro_invocation_is_not_shadowed_by_a_value() {
        let l = FileLocals::build(&facts());
        let mut m = call("cb", "", 20);
        m.macro_call = true;
        assert!(!l.shadows_bare_call(&m));
        assert!(l.shadows_bare_call(&call("cb", "", 20)));
    }

    #[test]
    fn a_local_that_holds_an_import_is_left_to_the_module_resolver() {
        // `const cb = require('./cb')`: the declaration (from byte 5) makes
        // the import (at byte 7); its scope starts after it (byte 10).
        let mut f = facts();
        f.locals[0].decl_byte = 5;
        f.imports.push(cgg_core::ImportRecord {
            kind: "import".into(),
            path: "./cb".into(),
            alias: "cb".into(),
            site_line: 1,
            site_byte: 7,
        });
        let l = FileLocals::build(&f);
        assert!(!l.shadows_bare_call(&call("cb", "", 20)));
        // Near miss: a parameter named like a file-level import still
        // shadows it (the import is outside its declaration).
        let mut f = facts();
        f.imports.push(cgg_core::ImportRecord {
            kind: "import".into(),
            path: "./c".into(),
            alias: "c".into(),
            site_line: 1,
            site_byte: 0,
        });
        let l = FileLocals::build(&f);
        assert!(l.shadows_receiver_head(&call("json", "c", 20)));
    }
}
