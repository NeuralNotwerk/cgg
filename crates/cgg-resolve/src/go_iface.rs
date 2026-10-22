//! Go's implicit interfaces (WS31).
//!
//! Go has no `implements`: a named type satisfies an interface when its
//! method set holds every method the interface names. A call through an
//! interface value resolves at most to the interface's method
//! declaration (bodiless, `abstract`), so the concrete methods it runs
//! have no caller. This module reads the method sets off the graph — an
//! interface's declared methods plus those of the interfaces it embeds,
//! a type's value- and pointer-receiver methods plus those promoted
//! through its embedded fields — and pairs each interface with the
//! types that satisfy it, by method name and, where the signature shows
//! it, parameter count.
//!
//! Types and interfaces are keyed by package directory and name, so two
//! packages' same-named types never merge. A qualified embed
//! (`store.Driver`) is looked up in the directories named like its
//! package; a standard-library one (`io.Reader`) in
//! [`cgg_core::stdlib::GO_STDLIB_INTERFACES`]. An interface embedding
//! anything else is skipped: its method set is unknown, so nothing can
//! be shown to satisfy it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cgg_core::stdlib::GO_STDLIB_INTERFACES;
use cgg_core::{
    FileFacts,
    graph::{CallEdge, CallableKind, Confidence, Graph, Via},
    ids::{CallableId, FileId, ResolverId},
};

use crate::dispatch::type_stem;
use crate::names::owner_from_qn;

/// Depth of embedding followed, for interfaces and for promoted fields.
const EMBED_DEPTH: usize = 4;

/// (package directory, type name).
type Key = (String, String);
/// Method name -> parameter count, where the signature shows it.
type MethodSet = BTreeMap<String, Option<usize>>;
/// An interface's own method declarations: (name, declaration, arity).
type OwnIfaces = BTreeMap<Key, Vec<(String, CallableId, Option<usize>)>>;
/// A standard-library interface's methods.
type StdlibSet = &'static [(&'static str, usize)];
/// An interface's own declarations, and its full method set (`None`:
/// an embed is unknown).
type Iface = (Vec<(String, CallableId)>, Option<MethodSet>);
/// The embeds a type or interface declares, as written.
type Embeds = BTreeMap<Key, Vec<String>>;

/// A method a type has: the callables that body it (several when a
/// promoted name comes from more than one embedded type) and its
/// parameter count. An interface embedded in a struct contributes the
/// name with no callable.
#[derive(Clone, Default)]
struct Method {
    ids: Vec<CallableId>,
    arity: Option<usize>,
}

pub(crate) struct GoMethodSets {
    /// Every named type with methods, own and promoted.
    types: BTreeMap<Key, BTreeMap<String, Method>>,
    /// Every interface the tree declares methods for: its own method
    /// declarations, and its full method set (`None`: an embed is
    /// unknown).
    ifaces: BTreeMap<Key, Iface>,
    /// Method name -> the types that have it.
    by_name: HashMap<String, Vec<Key>>,
}

impl GoMethodSets {
    pub(crate) fn build(graph: &Graph, facts: &[FileFacts]) -> Self {
        let dirs: HashMap<FileId, String> = graph
            .files
            .values()
            .filter(|f| f.language == "go")
            .map(|f| (f.id, dir_of(&f.path)))
            .collect();
        let mut own_types: BTreeMap<Key, BTreeMap<String, Method>> = BTreeMap::new();
        let mut own_ifaces: OwnIfaces = BTreeMap::new();
        for c in graph.callables.values() {
            if c.language != "go" || c.synthetic || c.kind != CallableKind::Method {
                continue;
            }
            let (Some(dir), Some(owner)) =
                (dirs.get(&c.file), owner_from_qn(&c.qualified_name))
            else {
                continue;
            };
            let key = (dir.clone(), type_stem(owner).to_string());
            let arity = param_count(&c.signature_hint, &c.simple_name);
            if c.attributes.iter().any(|a| a == "abstract") {
                own_ifaces.entry(key).or_default().push((
                    c.simple_name.clone(),
                    c.id,
                    arity,
                ));
            } else {
                let m = own_types
                    .entry(key)
                    .or_default()
                    .entry(c.simple_name.clone())
                    .or_default();
                m.ids.push(c.id);
                m.arity = m.arity.or(arity);
            }
        }
        // Embeds, by declaring type.
        let mut embeds: BTreeMap<Key, Vec<String>> = BTreeMap::new();
        for f in facts.iter().filter(|f| f.language == "go") {
            let dir = dir_of(&f.path);
            for c in &f.classes {
                let slot = embeds
                    .entry((dir.clone(), type_stem(&c.class_qn).to_string()))
                    .or_default();
                for b in &c.base_types {
                    // `pkg.T` through the file's import of `pkg`: written
                    // as the import path, so the package is found by it.
                    let b = match b.split_once('.') {
                        Some((q, t)) if stdlib_named(b).is_none() => f
                            .imports
                            .iter()
                            .find(|i| {
                                let p = i.path.trim_matches('"');
                                i.alias == q || p.rsplit('/').next() == Some(q)
                            })
                            .map(|i| format!("{}.{t}", i.path.trim_matches('"')))
                            .unwrap_or_else(|| b.clone()),
                        _ => b.clone(),
                    };
                    if !slot.contains(&b) {
                        slot.push(b);
                    }
                }
            }
        }
        for v in embeds.values_mut() {
            v.sort();
        }
        // Directories by their last segment: a qualified embed
        // `pkg.T` names a type of a package `pkg`, by convention the
        // directory's name.
        let mut by_base: HashMap<&str, Vec<&str>> = HashMap::new();
        let all_dirs: BTreeSet<&String> = dirs.values().collect();
        for d in &all_dirs {
            let base = d.rsplit('/').next().unwrap_or(d);
            by_base.entry(base).or_default().push(d.as_str());
        }
        // A qualified embed's package: the directories named like it,
        // narrowed to those sharing the most trailing segments with the
        // import path it was written through.
        let resolve = |from_dir: &str, name: &str| -> Vec<Key> {
            match name.rsplit_once('.') {
                None => vec![(from_dir.to_string(), name.to_string())],
                Some((pkg, t)) => {
                    let segs: Vec<&str> = pkg.split('/').collect();
                    let base = segs.last().copied().unwrap_or(pkg);
                    let ds = by_base.get(base).map(Vec::as_slice).unwrap_or(&[]);
                    let score = |d: &str| {
                        d.split('/')
                            .rev()
                            .zip(segs.iter().rev())
                            .take_while(|(a, b)| a == *b)
                            .count()
                    };
                    let best = ds.iter().map(|d| score(d)).max().unwrap_or(0);
                    ds.iter()
                        .filter(|d| score(d) == best)
                        .map(|d| (d.to_string(), t.to_string()))
                        .collect()
                }
            }
        };

        // Interface method sets, embeds followed.
        fn iface_set(
            key: &Key,
            depth: usize,
            own: &OwnIfaces,
            embeds: &Embeds,
            resolve: &dyn Fn(&str, &str) -> Vec<Key>,
        ) -> Option<MethodSet> {
            if depth > EMBED_DEPTH {
                return None;
            }
            let mut set: MethodSet = BTreeMap::new();
            let mut known = false;
            if let Some(ms) = own.get(key) {
                known = true;
                for (n, _, a) in ms {
                    set.insert(n.clone(), *a);
                }
            }
            for e in embeds.get(key).map(Vec::as_slice).unwrap_or(&[]) {
                known = true;
                if let Some(ms) = stdlib_named(e) {
                    for (n, a) in ms {
                        set.insert(n.to_string(), Some(*a));
                    }
                    continue;
                }
                // Exactly one in-tree interface of that name.
                let found: Vec<MethodSet> = resolve(&key.0, e)
                    .iter()
                    .filter(|k| own.contains_key(*k) || embeds.contains_key(*k))
                    .filter_map(|k| iface_set(k, depth + 1, own, embeds, resolve))
                    .collect();
                if found.len() != 1 {
                    return None;
                }
                set.extend(found.into_iter().next().unwrap_or_default());
            }
            known.then_some(set)
        }
        let mut ifaces = BTreeMap::new();
        for (key, ms) in &own_ifaces {
            let full = iface_set(key, 0, &own_ifaces, &embeds, &resolve);
            let decls = ms.iter().map(|(n, id, _)| (n.clone(), *id)).collect();
            ifaces.insert(key.clone(), (decls, full));
        }

        // Type method sets, promoted methods added under own ones.
        fn promote(
            key: &Key,
            depth: usize,
            own_types: &BTreeMap<Key, BTreeMap<String, Method>>,
            own_ifaces: &OwnIfaces,
            embeds: &Embeds,
            resolve: &dyn Fn(&str, &str) -> Vec<Key>,
            out: &mut BTreeMap<String, Method>,
        ) {
            if depth > EMBED_DEPTH {
                return;
            }
            for e in embeds.get(key).map(Vec::as_slice).unwrap_or(&[]) {
                let add = |n: &str, m: Method, out: &mut BTreeMap<String, Method>| {
                    // A depth-0 type's own method shadows a promoted one.
                    out.entry(n.to_string()).or_insert(m);
                };
                if let Some(ms) = stdlib_named(e) {
                    for (n, a) in ms {
                        add(
                            n,
                            Method {
                                ids: Vec::new(),
                                arity: Some(*a),
                            },
                            out,
                        );
                    }
                    continue;
                }
                for k in resolve(&key.0, e) {
                    if let Some(ms) = own_types.get(&k) {
                        for (n, m) in ms {
                            add(n, m.clone(), out);
                        }
                    }
                    if let Some(ms) = own_ifaces.get(&k) {
                        for (n, _, a) in ms {
                            add(
                                n,
                                Method {
                                    ids: Vec::new(),
                                    arity: *a,
                                },
                                out,
                            );
                        }
                    }
                    promote(&k, depth + 1, own_types, own_ifaces, embeds, resolve, out);
                }
            }
        }
        let mut types: BTreeMap<Key, BTreeMap<String, Method>> = BTreeMap::new();
        let type_keys: BTreeSet<&Key> = own_types
            .keys()
            .chain(embeds.keys().filter(|k| !own_ifaces.contains_key(*k)))
            .collect();
        for key in type_keys {
            let mut ms = own_types.get(key).cloned().unwrap_or_default();
            promote(key, 0, &own_types, &own_ifaces, &embeds, &resolve, &mut ms);
            if !ms.is_empty() {
                types.insert(key.clone(), ms);
            }
        }
        let mut by_name: HashMap<String, Vec<Key>> = HashMap::new();
        for (k, ms) in &types {
            for n in ms.keys() {
                by_name.entry(n.clone()).or_default().push(k.clone());
            }
        }
        GoMethodSets {
            types,
            ifaces,
            by_name,
        }
    }

    /// Whether type `t`'s method set covers `set`.
    fn satisfies(&self, t: &Key, set: &MethodSet) -> bool {
        let Some(ms) = self.types.get(t) else {
            return false;
        };
        set.iter().all(|(n, a)| {
            ms.get(n)
                .is_some_and(|m| !matches!((a, m.arity), (Some(x), Some(y)) if *x != y))
        })
    }

    /// The types satisfying a method set, sorted.
    fn satisfying(&self, set: &MethodSet) -> Vec<&Key> {
        // The rarest name narrows the candidates.
        let Some(rare) = set
            .keys()
            .min_by_key(|n| self.by_name.get(*n).map_or(0, Vec::len))
        else {
            return Vec::new();
        };
        let mut out: Vec<&Key> = self
            .by_name
            .get(rare)
            .map(|ks| ks.iter().filter(|k| self.satisfies(k, set)).collect())
            .unwrap_or_default();
        out.sort();
        out
    }

    /// Declaration → implementation pairs: each in-tree interface method
    /// declaration and the same-named method of every type satisfying the
    /// interface (promoted ones included), sorted and deduplicated.
    pub(crate) fn implementations(&self) -> Vec<(CallableId, CallableId)> {
        let mut out: BTreeSet<(CallableId, CallableId)> = BTreeSet::new();
        for (decls, full) in self.ifaces.values() {
            let Some(full) = full else { continue };
            if full.is_empty() || stdlib_lookalike(full) {
                continue;
            }
            for t in self.satisfying(full) {
                let ms = &self.types[t];
                for (name, decl) in decls {
                    if let Some(m) = ms.get(name) {
                        for id in &m.ids {
                            if id != decl {
                                out.insert((*decl, *id));
                            }
                        }
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    /// The standard-library interface a Go method's type implements
    /// with it: `fmt.Stringer` for a `String() string`, `sort.Interface`
    /// for a `Less` beside `Len` and `Swap`. The first in table order.
    pub(crate) fn stdlib_contract(
        &self,
        graph: &Graph,
        id: CallableId,
    ) -> Option<&'static str> {
        let c = graph.callables.get(&id)?;
        let dir = dir_of(&graph.files.get(&c.file)?.path);
        let owner = type_stem(owner_from_qn(&c.qualified_name)?);
        let t = (dir, owner.to_string());
        GO_STDLIB_INTERFACES.iter().find_map(|(name, ms)| {
            if !ms.iter().any(|(n, _)| *n == c.simple_name) {
                return None;
            }
            let set: MethodSet =
                ms.iter().map(|(n, a)| (n.to_string(), Some(*a))).collect();
            self.satisfies(&t, &set).then_some(*name)
        })
    }
}

/// `Via::Dynamic` edges from each Go interface method declaration to the
/// same-named method of every type satisfying the interface.
pub fn go_interface_fanout(graph: &Graph, facts: &[FileFacts]) -> Vec<CallEdge> {
    if !graph
        .callables
        .values()
        .any(|c| c.language == "go" && c.attributes.iter().any(|a| a == "abstract"))
    {
        return Vec::new();
    }
    let resolver = ResolverId::new("dispatch:go-interface");
    let sets = GoMethodSets::build(graph, facts);
    sets.implementations()
        .into_iter()
        .map(|(decl, imp)| {
            let (site_line, site_byte) = graph
                .callables
                .get(&decl)
                .map(|n| (n.start_line, n.start_byte))
                .unwrap_or((0, 0));
            CallEdge {
                src: decl,
                dst: imp,
                site_line,
                site_byte,
                confidence: Confidence::Low,
                via: Via::Dynamic,
                resolver: resolver.clone(),
                weight: 1,
            }
        })
        .collect()
}

/// `Via::Dynamic` edges from a Go method call the resolvers left
/// unresolved (`s.driver.Get(k)`, whose receiver is a field the
/// resolvers cannot type) to the method declaration of each in-tree
/// interface naming that method that the calling file can see: one of
/// its own package, or of a package it imports. With the
/// declaration→implementation edges of [`go_interface_fanout`], a live
/// call through an interface value keeps the satisfying types' methods
/// live. A call through a package (`http.Get`), a value reference and
/// a call no candidate of that name was found for are left alone.
/// Dead-code only: these are guesses about a receiver's type, not
/// edges of the call graph.
pub fn go_interface_site_edges(graph: &Graph, facts: &[FileFacts]) -> Vec<CallEdge> {
    use cgg_core::audit::UnresolvedReason as R;
    let mut decls: HashMap<&str, Vec<(String, CallableId)>> = HashMap::new();
    let dirs: HashMap<FileId, String> = graph
        .files
        .values()
        .filter(|f| f.language == "go")
        .map(|f| (f.id, dir_of(&f.path)))
        .collect();
    for c in graph.callables.values() {
        if c.language == "go"
            && c.kind == CallableKind::Method
            && c.attributes.iter().any(|a| a == "abstract")
            && let Some(d) = dirs.get(&c.file)
        {
            decls
                .entry(c.simple_name.as_str())
                .or_default()
                .push((d.clone(), c.id));
        }
    }
    if decls.is_empty() {
        return Vec::new();
    }
    // File -> the import paths it imports, as segments.
    let imports: HashMap<FileId, Vec<Vec<&str>>> = facts
        .iter()
        .filter(|f| f.language == "go")
        .map(|f| {
            (
                f.file,
                f.imports
                    .iter()
                    .map(|i| i.path.trim_matches('"').split('/').collect())
                    .collect(),
            )
        })
        .collect();
    let all_dirs: Vec<&str> = {
        let mut v: Vec<&str> = dirs.values().map(String::as_str).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut import_dirs: HashMap<String, Vec<String>> = HashMap::new();
    let resolver = ResolverId::new("dispatch:go-interface-site");
    let mut out: BTreeSet<(CallableId, CallableId, u32, u32)> = BTreeSet::new();
    for u in &graph.unresolved {
        let Some(src) = u.src else { continue };
        if u.receiver_hint.is_empty()
            || !matches!(
                u.reason,
                R::FanoutCapExceeded { .. }
                    | R::CandidatesInOtherFiles { .. }
                    | R::AmbiguousInFile
                    | R::NoCandidateCrossFile
            )
        {
            continue;
        }
        let (Some(cands), Some(dir)) = (decls.get(u.name.as_str()), dirs.get(&u.file))
        else {
            continue;
        };
        // The declaring package must be the file's own, or one it
        // imports: the directory its import path names, the one sharing
        // the most trailing segments with it over the whole tree
        // (`…/app_a/internal/repository` is not
        // `…/app_b/internal/repository`).
        let mut visible = |d: &str| {
            d == dir
                || imports.get(&u.file).is_some_and(|ps| {
                    ps.iter().any(|p| {
                        import_dirs
                            .entry(p.join("/"))
                            .or_insert_with(|| best_dirs(p, &all_dirs))
                            .iter()
                            .any(|x| x == d)
                    })
                })
        };
        for (d, id) in cands {
            if visible(d) {
                out.insert((src, *id, u.site_line, u.site_byte));
            }
        }
    }
    out.into_iter()
        .map(|(src, dst, site_line, site_byte)| CallEdge {
            src,
            dst,
            site_line,
            site_byte,
            confidence: Confidence::Low,
            via: Via::Dynamic,
            resolver: resolver.clone(),
            weight: 1,
        })
        .collect()
}

/// The directories an import path names: those sharing the most trailing
/// segments with it (at least its last), over every Go directory.
fn best_dirs(path: &[&str], dirs: &[&str]) -> Vec<String> {
    let score = |d: &str| {
        d.split('/')
            .rev()
            .zip(path.iter().rev())
            .take_while(|(a, b)| a == *b)
            .count()
    };
    let best = dirs.iter().map(|d| score(d)).max().unwrap_or(0);
    if best == 0 {
        return Vec::new();
    }
    dirs.iter()
        .filter(|d| score(d) == best)
        .map(|d| d.to_string())
        .collect()
}

/// The method set of the standard-library interface an embed names.
fn stdlib_named(name: &str) -> Option<StdlibSet> {
    GO_STDLIB_INTERFACES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, m)| *m)
}

/// Whether an interface's methods are all those of one standard-library
/// interface (a local `stringer` with `String() string`, a `closer`):
/// every type with a `String` method satisfies it, so it says nothing
/// about which of them a call reaches. Their implementations carry the
/// standard-library contract instead (`stdlib_contract`).
fn stdlib_lookalike(set: &MethodSet) -> bool {
    GO_STDLIB_INTERFACES
        .iter()
        .any(|(_, ms)| set.keys().all(|n| ms.iter().any(|(m, _)| m == n)))
}

fn dir_of(p: &std::path::Path) -> String {
    p.parent()
        .map(|d| d.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// The parameter count of method `name` in its signature hint
/// (`Read(p []byte) (int, error)`, `func (r *T) Read(p []byte) …`): the
/// top-level comma-separated items of the list after the name. `a, b
/// int` is two. `None` when the list cannot be found.
fn param_count(sig: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    let start = loop {
        let i = from + sig[from..].find(name)?;
        let after = i + name.len();
        let boundary = i == 0
            || !sig[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if boundary && sig[after..].starts_with('(') {
            break after + 1;
        }
        from = after;
    };
    let mut depth = 0i32;
    let mut items = 0usize;
    let mut any = false;
    for c in sig[start..].chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' => return Some(if any { items + 1 } else { 0 }),
            ',' if depth == 0 => items += 1,
            c if !c.is_whitespace() => any = true,
            _ => {}
        }
        if depth > 0 {
            any = true;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::param_count;

    #[test]
    fn parameter_counts_read_from_signatures() {
        assert_eq!(param_count("Read(p []byte) (int, error)", "Read"), Some(1));
        assert_eq!(
            param_count("func (r *T) Read(p []byte) (n int, err error)", "Read"),
            Some(1)
        );
        assert_eq!(
            param_count("func (t T) Less(i, j int) bool", "Less"),
            Some(2)
        );
        assert_eq!(param_count("Len() int", "Len"), Some(0));
        assert_eq!(
            param_count(
                "func (s *S) Do(f func(a, b int) error, m map[string]int)",
                "Do"
            ),
            Some(2)
        );
        assert_eq!(param_count("func (s *S) ReadAll(x int)", "Read"), None);
    }
}
