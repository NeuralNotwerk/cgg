//! Rolling a call graph up to a coarser granularity.
//!
//! The problem this solves is a budget one. The primary consumer of
//! cgg's mermaid output is a coding agent reading it in a context
//! window, and a full graph of a real tree does not fit: this repo's own
//! `crates/` renders 2,095 callables and 4,982 edges into 256 KB of
//! mermaid. The information an agent usually wants at that size is not
//! "which function calls which" but "which *module* calls which", and
//! that view is 35x smaller.
//!
//! # Shape
//!
//! Rollup is a `Graph` -> `Graph` transform, deliberately, rather than a
//! mode inside a formatter. Every formatter renders a [`Graph`] and
//! nothing else, so one transform gives mermaid, json, dot and graphml
//! the feature at once, and putting it in the pipeline gives it to all
//! four front ends instead of only the CLI. It runs at the very end —
//! after `--filter`/`-n`, after `--exclude-*`, after dead-code marking —
//! so it composes with every existing way of narrowing a graph rather
//! than competing with them.
//!
//! # What a group node claims
//!
//! A group node stands for N callables and carries a [`RollupMeta`]
//! saying so. A group *edge* stands for N call sites and carries the
//! count in [`CallEdge::weight`]. Two aggregation rules are worth being
//! explicit about, because the honest answer differs between them:
//!
//! * **Edge confidence is the maximum over the folded edges.** A
//!   group-to-group edge asserts "at least one call exists between these
//!   groups", which is a disjunction: it is true if any member edge is
//!   true, so it is as strong as the *strongest* evidence. Taking the
//!   minimum would understate something the graph knows.
//! * **`unreferenced` is the minimum, and only when every member has
//!   it.** That mark is a conjunction — "nothing calls anything in
//!   here" — so one referenced member falsifies it for the whole group,
//!   and the group's confidence is that of its weakest member.
//!
//! # Determinism
//!
//! Groups are minted in first-appearance order over `graph.callables`,
//! which is an `IndexMap` and therefore already in a deterministic
//! order, and edges are folded in `graph.edges` order. Nothing here
//! iterates a `HashMap`. That is not a stylistic preference: `query.rs`
//! shipped a `HashMap`-ordered entry list whose non-determinism stayed
//! invisible until `--max-paths` started turning work away, and a
//! budget that picks a level is exactly the same kind of trigger.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use cgg_core::graph::{
    CallEdge, CallableNode, Confidence, FileRecord, Graph, RollupMeta,
};
use cgg_core::ids::{CallableId, FileId, ResolverId};

use crate::stable_ids::StableIds;

/// Manifest filenames that mark a directory as a package root, for
/// [`RollupLevel::Package`].
///
/// Deliberately a list of *filenames*, not a language mapping: the
/// question "what package is this file in?" is answered by the nearest
/// ancestor directory carrying a build manifest, whatever language wrote
/// it, and a polyglot monorepo answers it correctly for every language
/// at once.
const PACKAGE_MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    "setup.py",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "composer.json",
    "Gemfile",
    "mix.exs",
    "pubspec.yaml",
    "CMakeLists.txt",
    "*.csproj",
];

/// How coarse a rolled-up graph should be.
///
/// Ordered finest to coarsest as written, which is the order
/// [`ladder`] walks when a budget has to escalate. The ordering is a
/// *heuristic* about typical trees, not an invariant — `Module` can
/// produce more groups than `File` in a tree of large single-module
/// files — which is why [`fit`] measures each rung instead of trusting
/// the sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RollupLevel {
    /// No rollup: one node per callable, the default graph.
    Callable,
    /// One node per owning type (`crate::io::DiskStorage`). Free
    /// functions have no owning type and fall back to their module.
    Type,
    /// One node per namespace. A method's namespace is the one
    /// containing its *type*, so `crate::io::DiskStorage::put` groups
    /// under `crate::io` rather than under `crate::io::DiskStorage`.
    Module,
    /// One node per source file.
    File,
    /// One node per package: the nearest ancestor directory holding a
    /// build manifest. See [`PACKAGE_MANIFESTS`].
    Package,
    /// One node per directory, cut at `N` path segments below the
    /// analysis root. `dir:1` is usually the top-level layout.
    Dir(u8),
    /// One node per language.
    Language,
}

impl RollupLevel {
    /// The wire/CLI spelling, and what lands in [`RollupMeta::level`].
    pub fn as_str(&self) -> String {
        match self {
            Self::Callable => "callable".into(),
            Self::Type => "type".into(),
            Self::Module => "module".into(),
            Self::File => "file".into(),
            Self::Package => "package".into(),
            Self::Dir(n) => format!("dir:{n}"),
            Self::Language => "language".into(),
        }
    }

    /// Whether this level actually groups anything.
    pub fn is_rollup(&self) -> bool {
        !matches!(self, Self::Callable)
    }
}

impl fmt::Display for RollupLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl FromStr for RollupLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("dir:") {
            let n: u8 = rest.parse().map_err(|_| {
                format!("--rollup-by dir:N needs a small positive number, got {rest:?}")
            })?;
            if n == 0 {
                return Err("--rollup-by dir:0 would put every file in one \
                            group; use `language` if that is what you want"
                    .into());
            }
            return Ok(Self::Dir(n));
        }
        match s {
            "callable" | "none" => Ok(Self::Callable),
            "type" | "owner" => Ok(Self::Type),
            "module" => Ok(Self::Module),
            "file" => Ok(Self::File),
            "package" | "crate" => Ok(Self::Package),
            "dir" => Ok(Self::Dir(1)),
            "language" | "lang" => Ok(Self::Language),
            other => Err(format!(
                "unknown rollup level {other:?} — expected one of: callable, \
                 type, module, file, package, dir:N, language"
            )),
        }
    }
}

// Crosses the C ABI and the Python/Node boundaries as its CLI spelling,
// so one grammar serves every front end and `RunOptions` stays a plain
// serde document.
impl serde::Serialize for RollupLevel {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for RollupLevel {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let s = String::deserialize(d)?;
        s.parse().map_err(D::Error::custom)
    }
}

/// The escalation sequence a budget walks, coarsening left to right.
///
/// `Callable` is not in it: it is the input, not a rung. `Dir(3)` and
/// `Dir(2)` sit between `Package` and `Dir(1)` because a package cut is
/// more semantically meaningful than an arbitrary depth, but on a deep
/// tree it can still be too fine.
///
/// `Language` is last and is genuinely a rung, not a placeholder. It was
/// left out of the first draft as "too coarse to be useful", and the
/// result was a run that reported a budget as unmeetable while a
/// grouping that met it went untried — a wrong answer dressed as a
/// limitation. If the caller asked for a budget that only "rust calls
/// python" fits inside, that is the honest thing to hand back.
pub fn ladder() -> [RollupLevel; 8] {
    [
        RollupLevel::Type,
        RollupLevel::Module,
        RollupLevel::File,
        RollupLevel::Package,
        RollupLevel::Dir(3),
        RollupLevel::Dir(2),
        RollupLevel::Dir(1),
        RollupLevel::Language,
    ]
}

/// Parse a token budget: `100000`, `100k`, `1.5m`.
///
/// Case-insensitive, and `_` separators are allowed because a budget is
/// the kind of number people paste from a model's context-window spec.
pub fn parse_budget(s: &str) -> Result<u64, String> {
    let raw = s.trim().to_ascii_lowercase().replace('_', "");
    let (num, mult) = match raw.strip_suffix('k') {
        Some(n) => (n, 1_000f64),
        None => match raw.strip_suffix('m') {
            Some(n) => (n, 1_000_000f64),
            None => (raw.as_str(), 1f64),
        },
    };
    let v: f64 = num
        .parse()
        .map_err(|_| format!("--rollup takes a token budget like `100k`, got {s:?}"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("--rollup budget must be positive, got {s:?}"));
    }
    Ok((v * mult).round() as u64)
}

/// Estimated token count of a rendered artifact.
///
/// Two estimators, and the answer is the larger:
///
/// * `words * 2.5` — the usual prose rule of thumb.
/// * `bytes / 3.5` — what code-shaped text actually costs.
///
/// They disagree badly on mermaid, and the disagreement is not academic.
/// This repo's own graph is 13,338 words and 256 KB: the word estimator
/// says 33k tokens, the byte estimator says 73k, and the truth is nearer
/// the byte one because 40% of the output is `C8xlv8ubomr`-style base36
/// ids and `::`-dense qualified names, none of which tokenize like
/// English. Taking the max means `--rollup 100k` errs toward rolling up
/// slightly too eagerly rather than handing back 2.3x what was asked
/// for, which is the failure that actually costs the caller something.
///
/// It is still an estimate. No tokenizer ships in this binary and none
/// will: cgg is offline, deterministic and single-binary, and a real BPE
/// vocabulary is none of those things at the size it would add.
pub fn estimate_tokens(rendered: &str) -> u64 {
    let words = rendered.split_whitespace().count() as u64;
    let bytes = rendered.len() as u64;
    std::cmp::max(words * 5 / 2, bytes * 2 / 7)
}

/// One rung the budget search tried.
#[derive(Clone, Debug)]
pub struct Attempt {
    pub level: RollupLevel,
    pub nodes: usize,
    pub edges: usize,
    pub tokens: u64,
}

/// What [`fit`] decided, and why.
#[derive(Clone, Debug)]
pub struct Fitted {
    /// The graph to emit. Identical to the input when `level` is
    /// [`RollupLevel::Callable`].
    pub graph: Graph,
    pub level: RollupLevel,
    /// Every rung measured, finest first. Always non-empty: the first
    /// entry is the un-rolled graph.
    pub attempts: Vec<Attempt>,
    /// The budget could not be met even at the coarsest rung.
    pub over_budget: bool,
}

impl Fitted {
    /// The measurement for the level that was actually chosen.
    ///
    /// Looked up by level rather than taken as the last attempt. Those
    /// coincide on the common path — the rung that fits is the last one
    /// measured — but not when nothing fits: there the smallest rung
    /// wins, and it is usually not the last one tried. Reading
    /// `attempts.last()` reported the wrong level and the wrong token
    /// count in exactly the case a caller most needs both.
    pub fn chosen(&self) -> &Attempt {
        self.attempts
            .iter()
            .find(|a| a.level == self.level)
            .or_else(|| self.attempts.last())
            .expect("fit always records at least the un-rolled graph")
    }
}

/// Pick the finest granularity whose rendered output fits `budget`.
///
/// `floor` is where the search starts — `--rollup-by` sets it, so
/// `--rollup-by file --rollup 50k` means "file level at least, coarser
/// if 50k demands it". With no budget, `floor` is applied exactly and
/// nothing is measured beyond it.
///
/// Each rung is rendered rather than predicted. Rendering is cheap next
/// to analysis (the whole pipeline is ~330 ms on this repo; rendering
/// its mermaid is single-digit ms) and every rung after the first is
/// dramatically smaller than the one before, so the search costs about
/// one extra render of the full graph. Predicting instead would put a
/// second, drifting cost model next to the real formatter.
pub fn fit(
    graph: &Graph,
    render: &dyn Fn(&Graph) -> String,
    budget: Option<u64>,
    floor: Option<RollupLevel>,
    ids: &mut StableIds,
    roots: &[crate::stable_ids::IdRoot],
) -> Fitted {
    let mut attempts = Vec::new();

    // No budget: `--rollup-by` alone is an instruction, not a search.
    let Some(budget) = budget else {
        let level = floor.unwrap_or(RollupLevel::Callable);
        let out = if level.is_rollup() {
            apply(graph, level, ids, roots)
        } else {
            graph.clone()
        };
        let text = render(&out);
        attempts.push(Attempt {
            level,
            nodes: out.callables.len(),
            edges: out.edges.len(),
            tokens: estimate_tokens(&text),
        });
        return Fitted {
            graph: out,
            level,
            attempts,
            over_budget: false,
        };
    };

    // With a budget, the un-rolled graph is always measured first — both
    // because it may already fit and because the caller deserves to be
    // told what it would have cost.
    let base_level = floor.unwrap_or(RollupLevel::Callable);
    if !base_level.is_rollup() {
        let text = render(graph);
        let tokens = estimate_tokens(&text);
        attempts.push(Attempt {
            level: RollupLevel::Callable,
            nodes: graph.callables.len(),
            edges: graph.edges.len(),
            tokens,
        });
        if tokens <= budget {
            return Fitted {
                graph: graph.clone(),
                level: RollupLevel::Callable,
                attempts,
                over_budget: false,
            };
        }
    }

    let rungs: Vec<RollupLevel> = ladder()
        .into_iter()
        .filter(|l| !base_level.is_rollup() || *l >= base_level)
        .collect();

    // Smallest rung seen so far, for the case where none of them fit.
    let mut best: Option<(Graph, RollupLevel, u64)> = None;
    for level in rungs {
        let out = apply(graph, level, ids, roots);
        let text = render(&out);
        let tokens = estimate_tokens(&text);
        attempts.push(Attempt {
            level,
            nodes: out.callables.len(),
            edges: out.edges.len(),
            tokens,
        });
        if tokens <= budget {
            return Fitted {
                graph: out,
                level,
                attempts,
                over_budget: false,
            };
        }
        if best.as_ref().is_none_or(|(_, _, t)| tokens < *t) {
            best = Some((out, level, tokens));
        }
    }

    // Nothing fit. Return the *smallest* thing measured, not the coarsest
    // one tried — they are usually the same, but not always: a rollup
    // adds a header banner and a per-node member-count tag, so on a graph
    // small enough for that overhead to dominate, a coarser cut can be
    // bigger than a finer one and even bigger than the input. Handing
    // back something larger than the un-rolled graph would be a strictly
    // worse answer to "keep this under N tokens" than doing nothing.
    let baseline = attempts
        .first()
        .filter(|a| !a.level.is_rollup())
        .map(|a| a.tokens);
    match best {
        Some((out, level, tokens)) if baseline.is_none_or(|b| tokens < b) => Fitted {
            graph: out,
            level,
            attempts,
            over_budget: true,
        },
        _ => Fitted {
            graph: graph.clone(),
            level: RollupLevel::Callable,
            attempts,
            over_budget: true,
        },
    }
}

/// Fold `graph` to one node per group at `level`.
///
/// Panics on [`RollupLevel::Callable`], which is not a grouping — the
/// callers above check `is_rollup()` first.
pub fn apply(
    graph: &Graph,
    level: RollupLevel,
    ids: &mut StableIds,
    roots: &[crate::stable_ids::IdRoot],
) -> Graph {
    assert!(level.is_rollup(), "callable level is not a grouping");
    let level_name = level.as_str();
    let mut pkg = PackageProbe::default();

    // Pass 1: assign every callable to a group, in `callables` order.
    // `IndexMap` iteration is insertion order, and insertion order comes
    // from the deterministic file walk, so group ids are stable.
    let mut group_of: HashMap<CallableId, usize> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut members: Vec<Vec<CallableId>> = Vec::new();

    for (id, node) in &graph.callables {
        let key = group_key(graph, node, level, roots, &mut pkg);
        let g = *index.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            members.push(Vec::new());
            order.len() - 1
        });
        group_of.insert(*id, g);
        members[g].push(*id);
    }

    // Pass 2: fold edges. Self-edges become a per-group counter instead
    // of an arrow that points at its own box.
    let mut internal: Vec<u32> = vec![0; order.len()];
    let mut edge_order: Vec<(usize, usize, cgg_core::graph::Via)> = Vec::new();
    let mut edge_index: HashMap<(usize, usize, cgg_core::graph::Via), usize> =
        HashMap::new();
    let mut folded: Vec<(u32, Confidence)> = Vec::new();

    for e in &graph.edges {
        let (Some(&s), Some(&d)) = (group_of.get(&e.src), group_of.get(&e.dst)) else {
            // An edge whose endpoint is not in `callables` is already
            // inconsistent; dropping it here matches `prune`.
            continue;
        };
        if s == d {
            internal[s] = internal[s].saturating_add(e.weight);
            continue;
        }
        let key = (s, d, e.via.clone());
        let slot = *edge_index.entry(key.clone()).or_insert_with(|| {
            edge_order.push(key);
            folded.push((0, Confidence::Low));
            edge_order.len() - 1
        });
        folded[slot].0 = folded[slot].0.saturating_add(e.weight);
        // Disjunction: the aggregate is as strong as its best evidence.
        if conf_rank(e.confidence) > conf_rank(folded[slot].1) {
            folded[slot].1 = e.confidence;
        }
    }

    // Pass 3: mint the nodes.
    let mut out = Graph::new();
    let mut group_ids: Vec<CallableId> = Vec::with_capacity(order.len());
    let mut group_file_ids: Vec<FileId> = Vec::with_capacity(order.len());

    for (g, key) in order.iter().enumerate() {
        let ms = &members[g];
        let first = &graph.callables[&ms[0]];

        // A group that folded exactly one node, and folded nothing about
        // it — a framework-entry node, which is never grouped — passes
        // through as itself. Wrapping it in a group node would rename a
        // real node after its own key and drop the `framework_entry`
        // marker that makes it legible.
        if ms.len() == 1 && first.framework_entry.is_some() {
            let mut node = first.clone();
            let fid = ensure_file(&mut out, graph, node.file);
            node.file = fid;
            group_ids.push(node.id);
            group_file_ids.push(fid);
            out.add_callable(node);
            continue;
        }

        let mut langs: Vec<String> = ms
            .iter()
            .map(|m| graph.callables[m].language.clone())
            .collect();
        langs.sort();
        langs.dedup();
        let mut files: Vec<FileId> = ms.iter().map(|m| graph.callables[m].file).collect();
        files.sort();
        files.dedup();

        // A group that lives entirely in one file keeps that file's
        // record — blake3, line count and all — because it is still
        // true of the group. One that spans files gets a synthetic
        // record named for the group, so `node.file` always resolves.
        let file_id = if files.len() == 1 {
            ensure_file(&mut out, graph, files[0])
        } else {
            let fid = ids.file(&format!("<rollup:{level_name}>/{key}"));
            if !out.files.contains_key(&fid) {
                out.add_file(FileRecord {
                    id: fid,
                    path: PathBuf::from(key),
                    language: if langs.len() == 1 {
                        langs[0].clone()
                    } else {
                        "mixed".into()
                    },
                    detected_via: format!("rollup:{level_name}"),
                    blake3: "0".repeat(64),
                    size_bytes: 0,
                    lines: 0,
                    parse_ms: 0.0,
                    parse_status: "synthetic".into(),
                    test_role: None,
                    ..Default::default()
                });
            }
            fid
        };

        let unref: Vec<Confidence> = ms
            .iter()
            .filter_map(|m| graph.callables[m].unreferenced)
            .collect();
        // Conjunction: one referenced member falsifies the claim for the
        // whole group, and a group that survives is only as certain as
        // its least certain member.
        let unreferenced = if unref.len() == ms.len() && !ms.is_empty() {
            unref.iter().copied().min_by_key(|c| conf_rank(*c))
        } else {
            None
        };

        let id = ids.rollup_group(&level_name, key);
        let simple = key
            .rsplit(['/', ':', '.'])
            .next()
            .unwrap_or(key)
            .to_string();
        out.add_callable(CallableNode {
            id,
            qualified_name: key.clone(),
            simple_name: simple,
            // Inheriting the first member's kind labelled a directory a
            // `method` whenever the first callable in it happened to be
            // one — a lie to every consumer that filters on `kind`.
            kind: cgg_core::graph::CallableKind::Group,
            language: if langs.len() == 1 {
                langs[0].clone()
            } else {
                "mixed".into()
            },
            file: file_id,
            start_line: 0,
            end_line: 0,
            start_byte: 0,
            end_byte: 0,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: Vec::new(),
            synthetic: true,
            trait_impl_target: None,
            unreferenced,
            framework_entry: None,
            rollup: Some(RollupMeta {
                level: level_name.clone(),
                members: ms.len() as u32,
                files: files.len() as u32,
                languages: langs,
                internal_calls: internal[g],
                unreferenced_members: unref.len() as u32,
            }),
            ..Default::default()
        });
        group_ids.push(id);
        group_file_ids.push(file_id);
    }

    // Pass 4: emit the folded edges, in first-occurrence order.
    let resolver = ResolverId::new("rollup");
    for (slot, (s, d, via)) in edge_order.into_iter().enumerate() {
        let (weight, confidence) = folded[slot];
        out.add_edge(CallEdge {
            src: group_ids[s],
            dst: group_ids[d],
            // An aggregate edge has no single call site. Carrying the
            // first member's line would read as "the" call site to
            // anyone who did not check `weight`.
            site_line: 0,
            site_byte: 0,
            confidence,
            via,
            resolver: resolver.clone(),
            weight,
        });
    }

    // Whole-run counters describe the analysis, not this view of it, and
    // survive a rollup exactly as they survive `prune`.
    out.metrics = graph.metrics.clone();
    out
}

fn conf_rank(c: Confidence) -> u8 {
    match c {
        Confidence::High => 2,
        Confidence::Medium => 1,
        Confidence::Low => 0,
    }
}

/// Copy a file record across on first use.
fn ensure_file(out: &mut Graph, src: &Graph, id: FileId) -> FileId {
    if !out.files.contains_key(&id)
        && let Some(rec) = src.files.get(&id)
    {
        out.files.insert(id, rec.clone());
    }
    id
}

/// The group key for one callable.
///
/// Two node populations get their own rules, because the generic ones
/// produce nonsense for them:
///
/// * **`<framework-entry>` nodes are never grouped.** They all share one
///   sentinel file, so any path-based level would collapse every entry
///   in the tree into a single node — destroying exactly the thing an
///   entry node exists to say. They key on their own name and pass
///   through.
/// * **`<external>` / `<stdlib>` exit nodes group by dependency.**
///   They share a sentinel file too, but here collapsing is the *useful*
///   answer: `<external>::tree_sitter::Node` is a better summary than a
///   node per called method. At `dir`/`package`/`language` coarseness
///   they collapse the rest of the way, into one `<external>` bucket.
fn group_key(
    graph: &Graph,
    node: &CallableNode,
    level: RollupLevel,
    roots: &[crate::stable_ids::IdRoot],
    pkg: &mut PackageProbe,
) -> String {
    if node.framework_entry.is_some() {
        return node.qualified_name.clone();
    }
    let path = graph
        .files
        .get(&node.file)
        .map(|f| f.path.as_path())
        .unwrap_or(Path::new(""));
    let sentinel = path.to_str().is_some_and(|p| p.starts_with('<'));
    if sentinel {
        return match level {
            RollupLevel::Type | RollupLevel::Module | RollupLevel::File => {
                split_outside_brackets(&node.qualified_name)
                    .map(|(prefix, _)| prefix.to_string())
                    .unwrap_or_else(|| node.qualified_name.clone())
            }
            _ => path.to_string_lossy().into_owned(),
        };
    }

    match level {
        RollupLevel::Callable => unreachable!("checked by the caller"),
        RollupLevel::Language => node.language.clone(),
        RollupLevel::Type => owner_path(&node.qualified_name),
        RollupLevel::Module => {
            let owner = owner_path(&node.qualified_name);
            // A method's *module* is the one containing its type, so
            // strip one more segment for the kinds that have a type
            // owner. `kind` is the only signal available: nothing in a
            // qualified name distinguishes `mod::Type` from `mod::sub`.
            if has_type_owner(node.kind) {
                split_outside_brackets(&owner)
                    .map(|(prefix, _)| prefix.to_string())
                    .unwrap_or(owner)
            } else {
                owner
            }
        }
        RollupLevel::File => crate::stable_ids::id_path(path, roots),
        RollupLevel::Dir(n) => {
            let rel = crate::stable_ids::id_path(path, roots);
            let segs: Vec<&str> = rel.split('/').collect();
            // A file shallower than the cut keeps its own directory
            // rather than being promoted to the root bucket.
            let take = std::cmp::min(n as usize, segs.len().saturating_sub(1));
            if take == 0 {
                ".".to_string()
            } else {
                segs[..take].join("/")
            }
        }
        RollupLevel::Package => pkg.package_of(path, roots),
    }
}

/// True for the kinds whose qualified name ends in `Type::method`.
fn has_type_owner(kind: cgg_core::graph::CallableKind) -> bool {
    use cgg_core::graph::CallableKind as K;
    matches!(
        kind,
        K::Method | K::Constructor | K::Destructor | K::Property
    )
}

/// Split a qualified name at its rightmost separator that is **outside**
/// any `<...>`.
///
/// `cgg_resolve::names::split_last_segment` is a plain `rfind`, which is
/// right for the resolver — it only ever needs the trailing simple name,
/// and that is never inside brackets. A rollup key needs the segment
/// *before* that too, and there the difference is visible: splitting
/// `cgg::cli::<cgg_format::OutputFormat as From<Arg>>::from` naively
/// lands inside the impl wrapper and yields the group name
/// `cgg::cli::<cgg_format::OutputFormat`, a dangling bracket and a name
/// no reader recognises.
fn split_outside_brackets(qn: &str) -> Option<(&str, &str)> {
    let b = qn.as_bytes();
    let mut depth = 0i32;
    let mut i = b.len();
    while i > 0 {
        i -= 1;
        match b[i] {
            b'>' => depth += 1,
            b'<' => depth -= 1,
            b':' if depth <= 0 && i > 0 && b[i - 1] == b':' => {
                return Some((&qn[..i - 1], &qn[i + 1..]));
            }
            b'.' if depth <= 0 => return Some((&qn[..i], &qn[i + 1..])),
            _ => {}
        }
    }
    None
}

/// The qualified name with its last segment removed and any trait-impl
/// wrapper reduced to the bare implementing type:
/// `crate::io::<D as S>::put` -> `crate::io::D`.
///
/// The owner is reduced to its *bare* name while the module path in
/// front of it is kept whole. Keeping the path is what stops two
/// `Parser` types in different modules from sharing a group; reducing
/// the owner is what stops `<cgg_format::OutputFormat as From<..>>`
/// from grouping under a name containing another crate's whole path.
fn owner_path(qn: &str) -> String {
    let Some((prefix, _)) = split_outside_brackets(qn) else {
        return qn.to_string();
    };
    match split_outside_brackets(prefix) {
        Some((head, last)) => {
            let norm = cgg_resolve::names::normalize_owner(last);
            // `normalize_owner` leaves `a::B` alone when the wrapper held
            // a path; only the final segment names the type.
            let bare = norm.rsplit("::").next().unwrap_or(norm);
            if bare.is_empty() {
                head.to_string()
            } else {
                format!("{head}::{bare}")
            }
        }
        None => {
            let norm = cgg_resolve::names::normalize_owner(prefix);
            norm.rsplit("::").next().unwrap_or(norm).to_string()
        }
    }
}

/// Nearest-ancestor package lookup, memoized per directory.
///
/// This is the one level that reads the filesystem — a `metadata` call
/// per candidate directory, cached, never a write. It is also the one
/// level that can fail: `--from-graph` replays a graph whose tree may
/// not exist on this machine, and a checkout that has moved answers
/// nothing. The fallback is `dir:1`, and [`Self::fell_back`] lets the
/// caller say so instead of silently handing back a differently-shaped
/// graph than the flag named.
#[derive(Debug, Default)]
struct PackageProbe {
    cache: HashMap<PathBuf, Option<PathBuf>>,
    fell_back: bool,
}

impl PackageProbe {
    fn package_of(&mut self, path: &Path, roots: &[crate::stable_ids::IdRoot]) -> String {
        let dir = path.parent().unwrap_or(Path::new(""));
        match self.nearest_manifest_dir(dir) {
            Some(found) => {
                let rel = crate::stable_ids::id_path(&found, roots);
                if rel.is_empty() { ".".into() } else { rel }
            }
            None => {
                self.fell_back = true;
                let rel = crate::stable_ids::id_path(path, roots);
                rel.split('/').next().unwrap_or(".").to_string()
            }
        }
    }

    fn nearest_manifest_dir(&mut self, dir: &Path) -> Option<PathBuf> {
        if let Some(hit) = self.cache.get(dir) {
            return hit.clone();
        }
        let answer = if dir.as_os_str().is_empty() {
            None
        } else if has_manifest(dir) {
            Some(dir.to_path_buf())
        } else {
            match dir.parent() {
                Some(parent) => self.nearest_manifest_dir(parent),
                None => None,
            }
        };
        self.cache.insert(dir.to_path_buf(), answer.clone());
        answer
    }
}

fn has_manifest(dir: &Path) -> bool {
    PACKAGE_MANIFESTS.iter().any(|m| {
        if let Some(ext) = m.strip_prefix("*.") {
            std::fs::read_dir(dir).is_ok_and(|rd| {
                rd.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == ext))
            })
        } else {
            dir.join(m).is_file()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, Via};

    #[test]
    fn every_level_the_cli_accepts_is_reachable_from_the_ladder() {
        // REGRESSION: `language` was missing, so a budget only it could
        // meet was reported unmeetable while the grouping that met it was
        // never tried.
        let rungs: Vec<RollupLevel> = ladder().into_iter().collect();
        for l in [
            RollupLevel::Type,
            RollupLevel::Module,
            RollupLevel::File,
            RollupLevel::Package,
            RollupLevel::Language,
        ] {
            assert!(rungs.contains(&l), "{l} is not on the escalation ladder");
        }
    }

    #[test]
    fn budget_suffixes() {
        assert_eq!(parse_budget("100000").unwrap(), 100_000);
        assert_eq!(parse_budget("100k").unwrap(), 100_000);
        assert_eq!(parse_budget("100K").unwrap(), 100_000);
        assert_eq!(parse_budget("1.5m").unwrap(), 1_500_000);
        assert_eq!(parse_budget("120_000").unwrap(), 120_000);
        assert!(parse_budget("banana").is_err());
        assert!(parse_budget("0").is_err());
        assert!(parse_budget("-5k").is_err());
    }

    #[test]
    fn level_round_trips_through_its_wire_form() {
        for s in ["callable", "type", "module", "file", "package", "language"] {
            let l: RollupLevel = s.parse().unwrap();
            assert_eq!(l.as_str(), s, "{s} did not round-trip");
        }
        assert_eq!("dir:2".parse::<RollupLevel>().unwrap(), RollupLevel::Dir(2));
        assert_eq!("dir:2".parse::<RollupLevel>().unwrap().as_str(), "dir:2");
        assert!("dir:0".parse::<RollupLevel>().is_err());
        assert!("modul".parse::<RollupLevel>().is_err());
    }

    #[test]
    fn owner_path_keeps_the_module_and_normalizes_impl_wrappers() {
        // The whole reason `owner_from_qn` is not enough: two `Parser`
        // types in different modules must not land in one group.
        assert_eq!(owner_path("a::b::Parser::new"), "a::b::Parser");
        assert_eq!(owner_path("c::d::Parser::new"), "c::d::Parser");
        assert_eq!(
            owner_path("crate::io::<Disk as Store>::put"),
            "crate::io::Disk"
        );
        assert_eq!(owner_path("crate::query::apply"), "crate::query");
        // REGRESSION: a naive rfind splits inside the impl wrapper and
        // produces `cgg::cli::<cgg_format::OutputFormat` — a dangling
        // bracket, and a group name for a type nobody wrote.
        assert_eq!(
            owner_path("cgg::cli::<cgg_format::OutputFormat as From<Arg>>::from"),
            "cgg::cli::OutputFormat"
        );
        assert_eq!(owner_path("m::Map<K, V>::insert"), "m::Map");
    }

    #[test]
    fn the_word_estimator_alone_would_understate_mermaid() {
        // Node ids are pure byte cost and no word cost, which is the
        // asymmetry `estimate_tokens` exists to survive.
        let ids = "  C8xlv8ubomr --> Cxmpa2intfx\n".repeat(500);
        let words = ids.split_whitespace().count() as u64 * 5 / 2;
        assert!(
            estimate_tokens(&ids) > words,
            "byte estimator must win on id-dense text"
        );
    }

    fn fixture() -> Graph {
        let mut g = Graph::new();
        for (fid, path) in [(0u32, "a/one.rs"), (1, "a/two.rs"), (2, "b/three.rs")] {
            g.add_file(FileRecord {
                id: FileId::new(fid),
                path: PathBuf::from(path),
                language: "rust".into(),
                detected_via: "ext".into(),
                blake3: "0".repeat(64),
                size_bytes: 1,
                lines: 1,
                parse_ms: 0.0,
                parse_status: "ok".into(),
                ..Default::default()
            });
        }
        let mk = |g: &mut Graph, id: u32, qn: &str, file: u32, kind: CallableKind| {
            g.add_callable(CallableNode {
                id: CallableId::new(id),
                qualified_name: qn.into(),
                simple_name: qn.rsplit("::").next().unwrap().into(),
                kind,
                language: "rust".into(),
                file: FileId::new(file),
                ..Default::default()
            });
        };
        mk(&mut g, 0, "a::one::Parser::new", 0, CallableKind::Method);
        mk(&mut g, 1, "a::one::Parser::step", 0, CallableKind::Method);
        mk(&mut g, 2, "a::two::helper", 1, CallableKind::Function);
        mk(&mut g, 3, "b::three::main", 2, CallableKind::Function);
        let e = |src: u32, dst: u32, conf: Confidence| CallEdge {
            src: CallableId::new(src),
            dst: CallableId::new(dst),
            confidence: conf,
            resolver: ResolverId::new("test"),
            ..Default::default()
        };
        g.add_edge(e(0, 1, Confidence::Low)); // intra-group at type level
        g.add_edge(e(0, 2, Confidence::Low));
        g.add_edge(e(1, 2, Confidence::High));
        g.add_edge(e(3, 0, Confidence::Medium));
        g
    }

    fn roll(level: RollupLevel) -> Graph {
        let mut ids = StableIds::new();
        apply(&fixture(), level, &mut ids, &[])
    }

    #[test]
    fn file_level_collapses_to_one_node_per_file() {
        let g = roll(RollupLevel::File);
        let names: Vec<&str> = g
            .callables
            .values()
            .map(|c| c.qualified_name.as_str())
            .collect();
        assert_eq!(names, ["a/one.rs", "a/two.rs", "b/three.rs"]);
        assert_eq!(g.callables.len(), 3);
    }

    #[test]
    fn folded_edges_carry_the_call_count_and_the_best_confidence() {
        let g = roll(RollupLevel::File);
        // one.rs -> two.rs folds two call sites, Low and High.
        let e = g
            .edges
            .iter()
            .find(|e| {
                g.callables[&e.src].qualified_name == "a/one.rs"
                    && g.callables[&e.dst].qualified_name == "a/two.rs"
            })
            .expect("one.rs -> two.rs");
        assert_eq!(e.weight, 2, "two call sites folded into one edge");
        assert_eq!(
            e.confidence,
            Confidence::High,
            "an aggregate edge is as strong as its best evidence, not its worst"
        );
        assert_eq!(e.site_line, 0, "an aggregate edge has no single call site");
    }

    #[test]
    fn intra_group_calls_become_a_count_not_a_self_loop() {
        let g = roll(RollupLevel::File);
        assert!(
            g.edges.iter().all(|e| e.src != e.dst),
            "no self-loops in a rolled-up graph"
        );
        let one = g
            .callables
            .values()
            .find(|c| c.qualified_name == "a/one.rs")
            .unwrap();
        assert_eq!(one.rollup.as_ref().unwrap().internal_calls, 1);
        assert_eq!(one.rollup.as_ref().unwrap().members, 2);
    }

    #[test]
    fn module_level_strips_the_type_for_methods_but_not_for_functions() {
        let g = roll(RollupLevel::Module);
        let names: Vec<&str> = g
            .callables
            .values()
            .map(|c| c.qualified_name.as_str())
            .collect();
        // Parser::new / Parser::step are methods -> module is `a::one`,
        // not `a::one::Parser`. `helper` is a free function -> `a::two`.
        assert!(names.contains(&"a::one"), "{names:?}");
        assert!(names.contains(&"a::two"), "{names:?}");
        assert!(names.contains(&"b::three"), "{names:?}");
    }

    #[test]
    fn type_level_keeps_the_owning_type() {
        let g = roll(RollupLevel::Type);
        let names: Vec<&str> = g
            .callables
            .values()
            .map(|c| c.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"a::one::Parser"), "{names:?}");
    }

    #[test]
    fn dir_level_cuts_at_n_segments() {
        let g = roll(RollupLevel::Dir(1));
        let names: Vec<&str> = g
            .callables
            .values()
            .map(|c| c.qualified_name.as_str())
            .collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn rolling_up_is_deterministic() {
        let a = roll(RollupLevel::Module);
        let b = roll(RollupLevel::Module);
        let key = |g: &Graph| {
            (
                g.callables
                    .values()
                    .map(|c| c.qualified_name.clone())
                    .collect::<Vec<_>>(),
                g.edges
                    .iter()
                    .map(|e| (e.src, e.dst, e.weight))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(key(&a), key(&b));
    }

    #[test]
    fn framework_entry_nodes_are_never_grouped() {
        let mut g = fixture();
        let fid = FileId::new(9);
        g.add_file(FileRecord {
            id: fid,
            path: PathBuf::from("<framework-entry>"),
            language: "framework-entry".into(),
            ..Default::default()
        });
        for (i, name) in ["<framework-entry>flask:/a", "<framework-entry>flask:/b"]
            .iter()
            .enumerate()
        {
            g.add_callable(CallableNode {
                id: CallableId::new(20 + i as u32),
                qualified_name: (*name).into(),
                simple_name: (*name).into(),
                language: "python".into(),
                file: fid,
                synthetic: true,
                framework_entry: Some(cgg_core::frameworks::TrustKind::Network),
                ..Default::default()
            });
        }
        let mut ids = StableIds::new();
        // `dir:1` would otherwise put both under the `<framework-entry>`
        // sentinel path and merge them into one node.
        let out = apply(&g, RollupLevel::Dir(1), &mut ids, &[]);
        let entries: Vec<&str> = out
            .callables
            .values()
            .filter(|c| c.framework_entry.is_some())
            .map(|c| c.qualified_name.as_str())
            .collect();
        assert_eq!(
            entries,
            ["<framework-entry>flask:/a", "<framework-entry>flask:/b"]
        );
    }

    #[test]
    fn a_group_is_unreferenced_only_when_every_member_is() {
        let mut g = fixture();
        // Mark one of one.rs's two callables.
        g.callables
            .get_mut(&CallableId::new(0))
            .unwrap()
            .unreferenced = Some(Confidence::High);
        let mut ids = StableIds::new();
        let out = apply(&g, RollupLevel::File, &mut ids, &[]);
        let one = out
            .callables
            .values()
            .find(|c| c.qualified_name == "a/one.rs")
            .unwrap();
        assert!(one.unreferenced.is_none(), "one live member falsifies it");
        assert_eq!(one.rollup.as_ref().unwrap().unreferenced_members, 1);

        g.callables
            .get_mut(&CallableId::new(1))
            .unwrap()
            .unreferenced = Some(Confidence::Low);
        let mut ids = StableIds::new();
        let out = apply(&g, RollupLevel::File, &mut ids, &[]);
        let one = out
            .callables
            .values()
            .find(|c| c.qualified_name == "a/one.rs")
            .unwrap();
        assert_eq!(
            one.unreferenced,
            Some(Confidence::Low),
            "the group is only as certain as its least certain member"
        );
    }

    #[test]
    fn distinct_via_kinds_between_the_same_groups_stay_separate() {
        let mut g = fixture();
        g.add_edge(CallEdge {
            src: CallableId::new(0),
            dst: CallableId::new(2),
            via: Via::Dynamic,
            confidence: Confidence::Low,
            resolver: ResolverId::new("test"),
            ..Default::default()
        });
        let mut ids = StableIds::new();
        let out = apply(&g, RollupLevel::File, &mut ids, &[]);
        let pairs: Vec<_> = out
            .edges
            .iter()
            .filter(|e| {
                out.callables[&e.src].qualified_name == "a/one.rs"
                    && out.callables[&e.dst].qualified_name == "a/two.rs"
            })
            .map(|e| (e.via.clone(), e.weight))
            .collect();
        assert_eq!(
            pairs.len(),
            2,
            "direct and dynamic must not merge: {pairs:?}"
        );
    }

    #[test]
    fn fit_leaves_a_graph_that_already_fits_alone() {
        let g = fixture();
        let mut ids = StableIds::new();
        let render =
            |g: &Graph| crate::emit::graph_to_string(g, crate::OutputFormat::Mermaid);
        let f = fit(&g, &render, Some(1_000_000), None, &mut ids, &[]);
        assert_eq!(f.level, RollupLevel::Callable);
        assert_eq!(f.graph.callables.len(), g.callables.len());
        assert!(!f.over_budget);
    }

    #[test]
    fn fit_escalates_until_it_fits_and_records_every_rung() {
        let g = fixture();
        let mut ids = StableIds::new();
        let render =
            |g: &Graph| crate::emit::graph_to_string(g, crate::OutputFormat::Mermaid);
        // Derived from the fixture rather than hard-coded: a literal
        // budget silently stops testing escalation the moment the
        // fixture's rendered size drifts past it.
        let base = estimate_tokens(&render(&g));
        let f = fit(&g, &render, Some(base / 2), None, &mut ids, &[]);
        assert!(f.attempts.len() >= 2, "must record what it tried");
        assert_eq!(f.attempts[0].level, RollupLevel::Callable);
        assert_eq!(f.attempts[0].tokens, base);
        // `chosen()` is the last rung measured, which on a fixture this
        // small may be over budget — what must hold is that every rung
        // was tried and none was silently skipped.
        let levels: Vec<String> = f.attempts.iter().map(|a| a.level.as_str()).collect();
        assert!(levels.contains(&"file".to_string()), "{levels:?}");
        assert!(levels.contains(&"dir:1".to_string()), "{levels:?}");
        assert!(
            f.graph.callables.len() <= g.callables.len(),
            "a rollup never returns more nodes than it was given"
        );
    }

    #[test]
    fn an_unmeetable_budget_never_returns_something_bigger_than_the_input() {
        // The fixture is small enough that the rollup banner and the
        // per-node member tags cost more than the folding saves. A
        // "budgeted" artifact larger than the unbudgeted one is the one
        // outcome that is strictly worse than not having the flag.
        let g = fixture();
        let mut ids = StableIds::new();
        let render =
            |g: &Graph| crate::emit::graph_to_string(g, crate::OutputFormat::Mermaid);
        let base = estimate_tokens(&render(&g));
        let f = fit(&g, &render, Some(1), None, &mut ids, &[]);
        assert!(f.over_budget);
        assert!(
            estimate_tokens(&render(&f.graph)) <= base,
            "budgeted output must not exceed the un-rolled output"
        );
    }

    #[test]
    fn chosen_reports_the_level_that_was_actually_returned() {
        // REGRESSION: `chosen()` read `attempts.last()`, so an
        // over-budget run named whichever rung was measured last rather
        // than the one it returned — the stderr line, the audit event and
        // the emitted graph disagreed with each other.
        let g = fixture();
        let mut ids = StableIds::new();
        let render =
            |g: &Graph| crate::emit::graph_to_string(g, crate::OutputFormat::Mermaid);
        let f = fit(&g, &render, Some(1), None, &mut ids, &[]);
        assert_eq!(f.chosen().level, f.level);
        assert_eq!(f.chosen().nodes, f.graph.callables.len());
        assert_eq!(f.chosen().edges, f.graph.edges.len());
    }

    #[test]
    fn an_unmeetable_budget_says_so_rather_than_pretending() {
        let g = fixture();
        let mut ids = StableIds::new();
        let render =
            |g: &Graph| crate::emit::graph_to_string(g, crate::OutputFormat::Mermaid);
        let f = fit(&g, &render, Some(1), None, &mut ids, &[]);
        assert!(f.over_budget, "1-token budget is not meetable");
        assert!(
            f.attempts.len() > 1,
            "it must have tried every rung before giving up"
        );
    }
}
