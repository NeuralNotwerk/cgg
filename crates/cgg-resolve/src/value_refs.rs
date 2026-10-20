//! Receiver-carrying value references (WS6, FINDINGS B3).
//!
//! A Java method reference (`this::m`, `Type::new`) or a C# method group
//! (`Select(F)`, `evt += this.H`) names a method without calling it. The
//! plugins record such a site as the call through that receiver it stands
//! for, marked [`RefRecord::value_ref`], so it is resolved by exactly the
//! machinery a call is — typed receivers, `super`, the calling class and
//! its bases, imports — rather than by bare name across the workspace.
//!
//! Two things differ from a call, and both live here:
//!
//! * **Before resolution** ([`prune`]): a value position holds a variable,
//!   field or property far more often than a method. A record whose name
//!   no callable of its language carries, or whose bare name is a local
//!   binding at the site, is data, not a reference to anything — it is
//!   dropped, as an unmatched [`cgg_core::VALUE_REF_HINT`] record always
//!   has been, instead of reaching the unresolved buckets as a call that
//!   does not exist.
//! * **After resolution** ([`tag_reference_edges`]): every edge drawn from
//!   such a site is a [`Via::Reference`] edge, so it is gated behind
//!   `--reference-edges` like every other reference.

use std::collections::{HashMap, HashSet};

use cgg_core::graph::{Graph, Via};
use cgg_core::ids::FileId;
use cgg_core::{FileFacts, RefRecord};

/// The callable name a value-ref record must match: its last path
/// segment (`a.b.Foo` for a constructor reference is `Foo`, Erlang
/// `m:f` is `f`, Clojure `alias/f` and `a.b/f` are `f`).
fn leaf(r: &RefRecord) -> &str {
    if let Some((ns, f)) = r.name.split_once('/')
        && !ns.is_empty()
        && !f.is_empty()
        && !ns.contains(':')
    {
        return f;
    }
    r.name.rsplit(['.', ':']).next().unwrap_or(&r.name)
}

/// Drop value-ref records that cannot name a callable: no callable of the
/// file's language has that simple name, or the bare name is a local
/// binding in scope at the site. Facts without a value-ref record are not
/// touched, so this is free on a run that records none.
pub fn prune(all_facts: &mut [FileFacts]) {
    if !all_facts
        .iter()
        .any(|f| f.references.iter().any(|r| r.value_ref))
    {
        return;
    }
    let mut names: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut members: HashMap<&str, HashSet<&str>> = HashMap::new();
    for f in all_facts.iter() {
        let set = names.entry(f.language.as_str()).or_default();
        set.extend(f.definitions.iter().map(|d| d.simple_name.as_str()));
        if !f.data_members.is_empty() {
            members
                .entry(f.language.as_str())
                .or_default()
                .extend(f.data_members.iter().map(String::as_str));
        }
    }
    // Objective-C++ (`.mm`) shares the C and C++ headers' classes: a field
    // a `.h` declares (`__weak FlutterViewController* view_controller_;`) is
    // data in the `.m`/`.mm` files that use it too.
    let c_cpp: HashSet<&str> = ["c", "cpp"]
        .iter()
        .filter_map(|l| members.get(l))
        .flat_map(|m| m.iter().copied())
        .collect();
    if !c_cpp.is_empty() {
        members.entry("objc").or_default().extend(c_cpp);
    }
    // C (WS27): what each file with a bare value can see through its
    // `#include` closure — the fields its own and those headers' structs
    // declare, and the functions they define or declare. A field the file
    // sees makes a bare name data, unless the file itself defines a
    // function of that name (GObject's `iface->shutdown = shutdown;`: C
    // has no member functions, where a bare name could be a field). A
    // field only some unrelated struct of the tree declares (musl's
    // `start`) does so only when no function of that name is visible:
    // `__clone(…, start, …)` beside a `static int start(void *)` names the
    // function. C++ and Objective-C keep the language-wide reading until
    // their locals are in the table: a member function's bare name is
    // its class's field, and their locals still read as values.
    let closures = crate::cross_file::c_include_closures(all_facts, &|f| {
        f.language == "c"
            && f.references
                .iter()
                .any(|r| r.value_ref && r.receiver_hint.is_empty())
    });
    let by_file: HashMap<cgg_core::FileId, &FileFacts> =
        all_facts.iter().map(|f| (f.file, f)).collect();
    // File → (fields it sees, functions it sees, functions it defines).
    type Seen = (HashSet<String>, HashSet<String>, HashSet<String>);
    let mut visible: HashMap<cgg_core::FileId, Seen> = HashMap::new();
    for (file, seen) in &closures {
        let mut fields = HashSet::new();
        let mut functions = HashSet::new();
        for id in std::iter::once(file).chain(seen.iter()) {
            let Some(f) = by_file.get(id) else { continue };
            fields.extend(f.data_members.iter().cloned());
            functions.extend(f.definitions.iter().map(|d| d.simple_name.clone()));
            functions.extend(f.unified_decls.iter().map(|(simple, _)| simple.clone()));
        }
        let own = by_file
            .get(file)
            .map(|f| {
                f.definitions
                    .iter()
                    .map(|d| d.simple_name.clone())
                    .collect()
            })
            .unwrap_or_default();
        visible.insert(*file, (fields, functions, own));
    }
    // Owned so the facts can be borrowed mutably below.
    let own = |m: HashMap<&str, HashSet<&str>>| -> HashMap<String, HashSet<String>> {
        m.into_iter()
            .map(|(l, s)| (l.to_string(), s.into_iter().map(str::to_string).collect()))
            .collect()
    };
    let names = own(names);
    let members = own(members);
    // Erlang: the arities of every clause by qualified name (`m:f`), for
    // the strict arity check below.
    // Objective-C: the C functions of the tree (no `Class::` owner). A
    // bare value in a `.m` file names one of them, never a method — a
    // variable, a property or an ivar shares a method's name far more
    // often than it is the method.
    let objc_functions: HashSet<&str> = all_facts
        .iter()
        .filter(|f| f.language == "objc")
        .flat_map(|f| f.definitions.iter())
        .filter(|d| !d.qualified_name.contains("::"))
        .map(|d| d.simple_name.as_str())
        .collect();
    let objc_functions: HashSet<String> =
        objc_functions.into_iter().map(str::to_string).collect();
    let mut erlang_clauses: HashMap<String, Vec<Option<u32>>> = HashMap::new();
    for f in all_facts.iter().filter(|f| f.language == "erlang") {
        for d in &f.definitions {
            erlang_clauses
                .entry(d.qualified_name.clone())
                .or_default()
                .push(crate::intra_file::clause_arity(
                    &d.signature_hint,
                    &d.simple_name,
                ));
        }
    }
    let empty = HashSet::new();
    for f in all_facts.iter_mut() {
        if !f.references.iter().any(|r| r.value_ref) {
            continue;
        }
        let known = names.get(&f.language).unwrap_or(&empty);
        // A bare name some struct or class of the language declares as a
        // data member is, inside a member function, that field far more
        // often than a function (`result.height = height`).
        let fields = members.get(&f.language).unwrap_or(&empty);
        let seen = (f.language == "c").then(|| visible.get(&f.file)).flatten();
        let is_field = |name: &str| match seen {
            Some((seen_fields, seen_functions, own_functions)) => {
                (seen_fields.contains(name) && !own_functions.contains(name))
                    || (fields.contains(name) && !seen_functions.contains(name))
            }
            None => fields.contains(name),
        };
        // Where a member read through a receiver is a field or getter read
        // (`tester.view`, `game.paused`), a member some class declares as
        // data is not a tear-off either.
        let member_reads = MEMBER_DATA_READ_LANGS.contains(&f.language.as_str());
        let locals = crate::scope::FileLocals::build(f);
        let defs = std::mem::take(&mut f.definitions);
        // Erlang: this module's clause arities by bare name.
        let own_clauses: Option<HashMap<&str, Vec<Option<u32>>>> =
            (f.language == "erlang").then(|| {
                let mut m: HashMap<&str, Vec<Option<u32>>> = HashMap::new();
                for d in &defs {
                    m.entry(d.simple_name.as_str()).or_default().push(
                        crate::intra_file::clause_arity(
                            &d.signature_hint,
                            &d.simple_name,
                        ),
                    );
                }
                m
            });
        let zig = f.language == "zig";
        let objc = f.language == "objc";
        let imports = std::mem::take(&mut f.imports);
        f.references.retain(|r| {
            !r.value_ref
                || (known.contains(leaf(r))
                    && (!zig || zig_value_in_scope(r, &defs, &imports))
                    && (!objc
                        || !r.receiver_hint.is_empty()
                        || objc_functions.contains(&r.name))
                    && own_clauses.as_ref().is_none_or(|own| {
                        erlang_value_names_a_clause(r, own, &erlang_clauses)
                    })
                    && !((r.receiver_hint.is_empty() || member_reads)
                        && is_field(&r.name))
                    && !shadowed(&locals, &defs, r))
        });
        f.definitions = defs;
        f.imports = imports;
    }
}

/// A Zig value names a declaration in scope: a bare name one of this
/// file (any container's) or a file-level alias of another file's
/// (`const fatal = protocol.fatal;`); a member through `@import` a file of
/// the tree (`mod.f`, a relative `.zig` path), `Self` or a type. A bare
/// name declared nowhere here is a local the locals spec did not reach,
/// never another file's function; a member of a package (`std.x`,
/// `builtin.target`) is that package's.
fn zig_value_in_scope(
    r: &RefRecord,
    defs: &[cgg_core::DefRecord],
    imports: &[cgg_core::ImportRecord],
) -> bool {
    if r.receiver_hint.is_empty() {
        // A capitalized bare name is a type (`writeInt(WORD, …)`) unless
        // this file defines a function of that name (a C extern,
        // `LLVMContextCreate`).
        if r.name.starts_with(|c: char| c.is_ascii_uppercase()) {
            return defs.iter().any(|d| d.simple_name == r.name);
        }
        return defs.iter().any(|d| d.simple_name == r.name)
            || imports.iter().any(|i| {
                matches!(i.kind.as_str(), "alias" | "import-member") && i.alias == r.name
            });
    }
    let head = r.receiver_hint.split('.').next().unwrap_or("");
    match imports
        .iter()
        .find(|i| i.kind == "import" && i.alias == head)
    {
        Some(i) => i.path.ends_with(".zig"),
        None => true,
    }
}

/// A local binding shadows the site's bare name — unless the binding's own
/// declaration defines a function of that name (`f = lambda: …`, `const
/// f = () => …`, a C# local function): then the local *is* that
/// function, and naming it names the function.
fn shadowed(
    locals: &crate::scope::FileLocals,
    defs: &[cgg_core::DefRecord],
    r: &RefRecord,
) -> bool {
    if !locals.shadows_bare_call(r) {
        return false;
    }
    let Some(crate::scope::Binding {
        start, end, decl, ..
    }) = locals.binding_decl_at(&r.name, r.site_byte)
    else {
        return true;
    };
    // A local function's binding starts where its definition does and
    // runs on to the end of its block (C# `void Handler() {…}` passed as
    // `Run(Handler)`, WS27). A parameter named like its own function
    // (`function a(a)`) spans exactly that function: it is not the
    // function.
    !defs.iter().any(|d| {
        d.simple_name == r.name
            && decl <= d.start_byte
            && (d.start_byte < start || (d.start_byte == start && d.end_byte < end))
    })
}

/// An Erlang function value names one function of one arity: `fun f/N`
/// a clause of `f/N` in its own module (file), `fun m:f/N` and an
/// `{m, f, [..]}` callback a clause of `m:f/N` in a module of the tree.
/// A value whose arity no clause has names no function here — never one
/// of another arity — and one through a module the tree does not declare
/// (`{error, badarg}`, `fun lists:map/2`) names nothing in it.
fn erlang_value_names_a_clause(
    r: &RefRecord,
    own: &HashMap<&str, Vec<Option<u32>>>,
    clauses: &HashMap<String, Vec<Option<u32>>>,
) -> bool {
    let arities = if r.receiver_hint.is_empty() {
        own.get(r.name.as_str())
    } else {
        clauses.get(&r.name)
    };
    arities.is_some_and(|a| r.arity.is_none() || a.contains(&r.arity))
}

/// Languages whose `x.m` in value position reads a field or getter far
/// more often than it tears off a method: a name some class declares as
/// data (`FileFacts::data_members`) is dropped through a receiver too.
const MEMBER_DATA_READ_LANGS: &[&str] = &["dart", "scala"];

/// Whether a definition is declared without a parameter list: the text
/// after its name in the signature (past a `[…]` type-parameter group)
/// does not open one. Scala's `def size: Int` is; `def size(): Int` is not.
fn declares_no_parameters(sig: &str, name: &str) -> bool {
    let Some(at) = sig.find(name) else {
        return false;
    };
    let mut rest = sig[at + name.len()..].trim_start();
    if rest.starts_with('[') {
        let mut depth = 0usize;
        let mut end = rest.len();
        for (i, ch) in rest.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = rest[end..].trim_start();
    }
    !rest.starts_with('(')
}

/// Drop paren-less records ([`RefRecord::paren_less`]) that cannot be a
/// call: no callable of the file's language is declared without a
/// parameter list under that name, or the bare name is a local binding at
/// the site. What remains is resolved as the call it is. Free on a run
/// whose plugins record none.
pub fn prune_paren_less(all_facts: &mut [FileFacts]) {
    if !all_facts
        .iter()
        .any(|f| f.references.iter().any(|r| r.paren_less))
    {
        return;
    }
    let mut names: HashMap<String, HashSet<String>> = HashMap::new();
    for f in all_facts.iter() {
        let set = names.entry(f.language.clone()).or_default();
        for d in &f.definitions {
            // A constructor is never called paren-less: its name written
            // bare is the type or its companion object.
            if d.variant != cgg_core::DefVariant::Constructor
                && declares_no_parameters(&d.signature_hint, &d.simple_name)
            {
                set.insert(d.simple_name.clone());
            }
        }
    }
    let empty = HashSet::new();
    for f in all_facts.iter_mut() {
        if !f.references.iter().any(|r| r.paren_less) {
            continue;
        }
        let known = names.get(&f.language).unwrap_or(&empty);
        let locals = crate::scope::FileLocals::build(f);
        let defs = std::mem::take(&mut f.definitions);
        f.references.retain(|r| {
            !r.paren_less || (known.contains(&r.name) && !shadowed(&locals, &defs, r))
        });
        // A kept paren-less site is a call of a parameterless `def`: a
        // value reference the plugin recorded on the same name and byte
        // (Scala eta-expansion) is that call, not a value.
        let calls: HashSet<(u32, &str)> = f
            .references
            .iter()
            .filter(|r| r.paren_less)
            .map(|r| (r.site_byte, r.name.as_str()))
            .collect();
        if !calls.is_empty() {
            let calls: HashSet<(u32, String)> =
                calls.into_iter().map(|(b, n)| (b, n.to_string())).collect();
            f.references.retain(|r| {
                !r.value_ref || !calls.contains(&(r.site_byte, r.name.clone()))
            });
        }
        f.definitions = defs;
    }
}

/// Re-tag every direct edge drawn from a value-ref site as a reference
/// edge. Sites are keyed by (file, byte); the plugins place a value-ref
/// site on the referenced name's own token, where no call starts, so a
/// call in the same expression is never re-tagged.
///
/// The edges are whatever the call through that receiver would have
/// drawn, at the same confidence: a `medium` edge here is the base-chain
/// or calling-class binding a `medium` call would be. Which value
/// positions are recorded at all is the plugins' decision — a C# member
/// read through a variable, or a Python attribute of one, is not, being
/// a property far more often than a method group.
pub fn tag_reference_edges(graph: &mut Graph, all_facts: &[FileFacts]) {
    let sites: HashSet<(FileId, u32)> = all_facts
        .iter()
        .flat_map(|f| {
            f.references
                .iter()
                .filter(|r| r.value_ref)
                .map(move |r| (f.file, r.site_byte))
        })
        .collect();
    if sites.is_empty() {
        return;
    }
    // A value reference can share its byte with a call that starts on
    // the same token: `f` in `f.bind(x)` is where the `bind` call
    // starts. At such a byte only the edges to what the value names are
    // reference edges; the call's own edges stay direct.
    let mut shared: HashMap<(FileId, u32), HashSet<&str>> = HashMap::new();
    for f in all_facts {
        let calls: HashSet<u32> = f
            .references
            .iter()
            .filter(|r| {
                !r.value_ref
                    && r.receiver_hint != cgg_core::VALUE_REF_HINT
                    && r.receiver_hint != cgg_core::STRING_REF_HINT
                    && sites.contains(&(f.file, r.site_byte))
            })
            .map(|r| r.site_byte)
            .collect();
        for r in f.references.iter().filter(|r| r.value_ref) {
            if calls.contains(&r.site_byte) {
                shared
                    .entry((f.file, r.site_byte))
                    .or_default()
                    .insert(leaf(r));
            }
        }
    }
    let node_of: HashMap<_, (FileId, &str)> = graph
        .callables
        .values()
        .map(|c| (c.id, (c.file, c.simple_name.as_str())))
        .collect();
    let mut retag: Vec<usize> = Vec::new();
    for (i, e) in graph.edges.iter().enumerate() {
        if e.via != Via::Direct {
            continue;
        }
        let Some((file, _)) = node_of.get(&e.src) else {
            continue;
        };
        let key = (*file, e.site_byte);
        if !sites.contains(&key) {
            continue;
        }
        let named = shared.get(&key).is_none_or(|names| {
            node_of
                .get(&e.dst)
                .is_some_and(|(_, dst)| names.contains(dst))
        });
        if named {
            retag.push(i);
        }
    }
    for i in retag {
        graph.edges[i].via = Via::Reference;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::DefRecord;

    fn facts(lang: &str, file: u32, defs: &[&str], refs: Vec<RefRecord>) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(file), format!("f{file}").into(), lang);
        f.definitions = defs
            .iter()
            .map(|n| DefRecord {
                simple_name: (*n).to_string(),
                qualified_name: (*n).to_string(),
                ..Default::default()
            })
            .collect();
        f.references = refs;
        f
    }

    fn vref(name: &str, recv: &str) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            value_ref: true,
            ..Default::default()
        }
    }

    #[test]
    fn paren_less_sites_survive_only_for_parameterless_callables() {
        let pl = |name: &str, recv: &str, byte: u32| RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_byte: byte,
            paren_less: true,
            ..Default::default()
        };
        let mut a = facts(
            "scala",
            1,
            &["go"],
            vec![
                pl("size", "b", 10),
                pl("m", "b", 20),
                pl("x", "", 30),
                pl("size", "", 40),
            ],
        );
        // `size` is the name of a local in scope at byte 40.
        a.locals = vec![cgg_core::LocalBinding {
            name: "size".into(),
            start_byte: 35,
            end_byte: 50,
            decl_byte: 32,
            param: false,
        }];
        let mut b = facts("scala", 2, &["size", "m"], vec![]);
        b.definitions[0].signature_hint = "def size: Int".into();
        b.definitions[1].signature_hint = "def m(): Int".into();
        let mut all = vec![a, b];
        prune_paren_less(&mut all);
        let kept: Vec<(&str, u32)> = all[0]
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.site_byte))
            .collect();
        // `m` takes a parameter list, `x` names nothing, the bare `size`
        // at 40 is the local.
        assert_eq!(kept, vec![("size", 10)]);
        assert!(declares_no_parameters("def get[T]: T", "get"));
        assert!(!declares_no_parameters("def get[T](k: K): T", "get"));
    }

    // WS22: a constructor is never what a bare `Foo` calls, even one
    // whose header reads as parameterless (`class Foo @Inject() (…)`); a
    // parameterless method is.
    #[test]
    fn a_paren_less_name_never_calls_a_constructor() {
        let pl = |name: &str, byte: u32| RefRecord {
            name: name.into(),
            site_byte: byte,
            paren_less: true,
            ..Default::default()
        };
        let a = facts("scala", 1, &["go"], vec![pl("Foo", 10), pl("tick", 20)]);
        let mut b = facts("scala", 2, &["Foo", "tick"], vec![]);
        b.definitions[0].signature_hint = "class Foo @Inject() (x: Int)".into();
        b.definitions[0].variant = cgg_core::DefVariant::Constructor;
        b.definitions[1].signature_hint = "def tick: Int".into();
        let mut all = vec![a, b];
        prune_paren_less(&mut all);
        let kept: Vec<&str> = all[0].references.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(kept, vec!["tick"]);
    }

    #[test]
    fn a_local_bound_to_its_own_function_does_not_shadow_it() {
        // `f = lambda: …` then `return f`: the local is the closure the
        // declaration defines, so naming it names the function.
        let mut a = facts(
            "python",
            1,
            &["main", "f"],
            vec![RefRecord {
                site_byte: 80,
                ..vref("f", "")
            }],
        );
        a.definitions[1].start_byte = 25;
        a.locals = vec![cgg_core::LocalBinding {
            name: "f".into(),
            start_byte: 40,
            end_byte: 100,
            decl_byte: 20,
            param: false,
        }];
        let mut all = vec![a];
        prune(&mut all);
        assert_eq!(all[0].references.len(), 1);
        // Near-miss: a local bound to something else still shadows.
        all[0].definitions[1].start_byte = 5;
        prune(&mut all);
        assert!(all[0].references.is_empty());
    }

    /// WS27: a C# local function's name is bound from its own start; the
    /// method group `Run(Handler)` names that function.
    #[test]
    fn a_local_function_named_as_a_value_is_kept() {
        let mut a = facts(
            "csharp",
            1,
            &["Main", "Handler"],
            vec![RefRecord {
                site_byte: 80,
                ..vref("Handler", "")
            }],
        );
        a.definitions[1].start_byte = 30;
        a.definitions[1].end_byte = 60;
        a.locals = vec![cgg_core::LocalBinding {
            name: "Handler".into(),
            start_byte: 30,
            end_byte: 100,
            decl_byte: 30,
            param: true,
        }];
        let mut all = vec![a];
        prune(&mut all);
        assert_eq!(all[0].references.len(), 1);
        // Near miss: a parameter named like its own function
        // (`function a(a) { g(a) }`) spans the function exactly.
        all[0].definitions[1].end_byte = 100;
        prune(&mut all);
        assert!(all[0].references.is_empty());
    }

    #[test]
    fn a_value_ref_naming_no_callable_is_dropped() {
        let mut all = vec![
            facts(
                "csharp",
                1,
                &["Run"],
                vec![vref("Count", "xs"), vref("Handle", "this")],
            ),
            facts("csharp", 2, &["Handle"], vec![]),
        ];
        prune(&mut all);
        let names: Vec<&str> =
            all[0].references.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Handle"]);
    }

    #[test]
    fn a_callable_of_another_language_does_not_keep_it() {
        let mut all = vec![
            facts("java", 1, &["run"], vec![vref("onItem", "this")]),
            facts("python", 2, &["onItem"], vec![]),
        ];
        prune(&mut all);
        assert!(all[0].references.is_empty());
    }

    #[test]
    fn a_constructor_reference_matches_by_its_last_segment() {
        let mut all = vec![facts("java", 1, &["Foo"], vec![vref("a.b.Foo", "")])];
        prune(&mut all);
        assert_eq!(all[0].references.len(), 1);
    }

    #[test]
    fn ordinary_calls_are_never_pruned() {
        let call = RefRecord {
            name: "nothing".into(),
            ..Default::default()
        };
        let mut all = vec![facts("java", 1, &["run"], vec![call, vref("x", "")])];
        prune(&mut all);
        assert_eq!(all[0].references.len(), 1);
        assert!(!all[0].references[0].value_ref);
    }

    /// WS26: in C/C++ a bare value-position name some struct or class
    /// declares as a data member is that field (`result.height = height`
    /// inside a member function), not a same-named function; a name only
    /// functions carry, or one written through a scope, is kept.
    #[test]
    fn a_bare_name_carried_by_a_data_member_is_not_a_reference() {
        let mut a = facts(
            "cpp",
            1,
            &["height", "cmp"],
            vec![vref("height", ""), vref("cmp", ""), vref("height", "Img")],
        );
        a.data_members = vec!["height".into()];
        let mut all = vec![a];
        prune(&mut all);
        let kept: Vec<(&str, &str)> = all[0]
            .references
            .iter()
            .map(|r| (r.receiver_hint.as_str(), r.name.as_str()))
            .collect();
        assert_eq!(kept, vec![("", "cmp"), ("Img", "height")]);
    }

    /// WS27: a C bare value is a field when a struct the file can see
    /// declares it (its own, or one in its `#include` closure). A field of
    /// that name only in an unrelated file hides it unless a function of
    /// that name is visible.
    #[test]
    fn a_c_field_hides_a_value_only_where_the_file_sees_it() {
        let mk = |id: u32, path: &str, defs: &[&str], refs: Vec<RefRecord>| {
            let mut f = facts("c", id, defs, refs);
            f.path = path.into();
            f
        };
        let mut a = mk(
            1,
            "src/a.c",
            &["shutdown"],
            vec![vref("start", ""), vref("shutdown", "")],
        );
        a.imports.push(cgg_core::ImportRecord {
            kind: "include".into(),
            path: "b.h".into(),
            alias: String::new(),
            site_line: 1,
            site_byte: 0,
        });
        let mut b = mk(2, "src/b.h", &[], vec![]);
        b.data_members = vec!["start".into(), "shutdown".into()];
        let c = mk(
            3,
            "src/c.c",
            &["handler"],
            vec![vref("handler", ""), vref("reader", "")],
        );
        let mut d = mk(4, "other/d.h", &["reader"], vec![]);
        d.data_members = vec!["handler".into(), "reader".into()];
        let mut all = vec![a, b, c, d];
        prune(&mut all);
        // The included field hides `start`; `shutdown`, also a field of the
        // included struct, is the function a.c itself defines
        // (`iface->shutdown = shutdown;`).
        let kept: Vec<&str> = all[0].references.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(kept, vec!["shutdown"]);
        // `handler` is a field of a struct c.c never includes, and c.c
        // defines a `handler`: the function. `reader` is such a field too,
        // but no function c.c can see carries the name: still data.
        let kept: Vec<&str> = all[2].references.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(kept, vec!["handler"]);
    }

    /// WS26: in Dart a member read through a receiver that names a field
    /// or getter some class declares is that read, not a tear-off; in C++
    /// only a bare name is filtered (`&Class::m` names a method).
    #[test]
    fn a_dart_member_read_naming_a_data_member_is_not_a_reference() {
        let run = |lang: &str| -> Vec<String> {
            let mut a = facts(
                lang,
                1,
                &["view", "handler"],
                vec![vref("view", "tester"), vref("handler", "h")],
            );
            a.data_members = vec!["view".into()];
            let mut all = vec![a];
            prune(&mut all);
            all[0].references.iter().map(|r| r.name.clone()).collect()
        };
        assert_eq!(run("dart"), vec!["handler"]);
        assert_eq!(run("cpp"), vec!["view", "handler"]);
    }

    /// WS26: a Scala name in a value position is recorded both as a
    /// paren-less site and as a value reference. When a parameterless
    /// `def` of the name exists the site is that call, and the value
    /// reference at the same byte is dropped; otherwise the value
    /// reference stands.
    #[test]
    fn a_kept_paren_less_call_drops_its_twin_value_reference() {
        let pl = |name: &str, byte: u32| RefRecord {
            name: name.into(),
            site_byte: byte,
            paren_less: true,
            ..Default::default()
        };
        let vr = |name: &str, byte: u32| RefRecord {
            name: name.into(),
            site_byte: byte,
            value_ref: true,
            ..Default::default()
        };
        let mut a = facts(
            "scala",
            1,
            &[],
            vec![
                pl("size", 10),
                vr("size", 10),
                pl("parse", 20),
                vr("parse", 20),
            ],
        );
        a.definitions = vec![
            DefRecord {
                simple_name: "size".into(),
                qualified_name: "C.size".into(),
                signature_hint: "def size: Int".into(),
                ..Default::default()
            },
            DefRecord {
                simple_name: "parse".into(),
                qualified_name: "C.parse".into(),
                signature_hint: "def parse(s: String): Int".into(),
                ..Default::default()
            },
        ];
        let mut all = vec![a];
        prune(&mut all);
        prune_paren_less(&mut all);
        let kept: Vec<(&str, bool)> = all[0]
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.value_ref))
            .collect();
        assert_eq!(kept, vec![("size", false), ("parse", true)]);
    }

    #[test]
    fn edges_from_value_ref_sites_become_reference_edges() {
        use crate::deadcode::testutil::{edge, graph_with, node};
        let mut g = graph_with(vec![
            node(0, "A.run", "run", "java"),
            node(1, "A.m", "m", "java"),
        ]);
        let mut at_ref = edge(0, 1);
        at_ref.site_byte = 40;
        let mut a_call = edge(0, 1);
        a_call.site_byte = 41;
        g.edges.push(at_ref);
        g.edges.push(a_call);
        let mut r = vref("m", "this");
        r.site_byte = 40;
        let f = facts("java", 0, &["run", "m"], vec![r]);
        tag_reference_edges(&mut g, &[f]);
        let vias: Vec<Via> = g.edges.iter().map(|e| e.via.clone()).collect();
        assert_eq!(vias, vec![Via::Reference, Via::Direct]);
    }

    /// `f.bind(x)`: the value reference to `f` and the `bind` call start
    /// at the same byte. Only the edge to `f` is a reference edge; the
    /// call's edge to a `bind` method stays direct.
    #[test]
    fn a_call_sharing_the_value_refs_byte_keeps_its_direct_edges() {
        use crate::deadcode::testutil::{edge, graph_with, node};
        let mut g = graph_with(vec![
            node(0, "main", "main", "javascript"),
            node(1, "f", "f", "javascript"),
            node(2, "Ctx.bind", "bind", "javascript"),
        ]);
        let mut to_f = edge(0, 1);
        to_f.site_byte = 40;
        let mut to_bind = edge(0, 2);
        to_bind.site_byte = 40;
        g.edges.push(to_f);
        g.edges.push(to_bind);
        let mut v = vref("f", "");
        v.site_byte = 40;
        let call = RefRecord {
            name: "bind".into(),
            receiver_hint: "f".into(),
            site_byte: 40,
            ..Default::default()
        };
        let f = facts("javascript", 0, &["main", "f"], vec![v, call]);
        tag_reference_edges(&mut g, &[f]);
        let vias: Vec<Via> = g.edges.iter().map(|e| e.via.clone()).collect();
        assert_eq!(vias, vec![Via::Reference, Via::Direct]);
    }
}
