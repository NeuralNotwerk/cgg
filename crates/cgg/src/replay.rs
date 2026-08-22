//! Re-querying a graph a previous run already produced.
//!
//! `-t json` writes the whole [`Graph`], and `Graph` has always derived
//! `Deserialize` — the document was a valid input long before anything
//! read one. This module is that reader plus the guardrails a saved
//! artifact needs, and nothing else: the query, exclusion and rollup
//! stages it runs are the same functions [`crate::analyze`] runs, called
//! in the same order.
//!
//! The point is cost. Analysis is the expensive half (parsing dominates
//! the wall clock and there is no cache by design), while slicing the
//! result is microseconds. One `cgg ./src -t json -o graph.json` can
//! then answer a dozen different questions without re-parsing anything.
//!
//! # What a replay cannot do
//!
//! A saved graph is the *post-query* graph of the run that wrote it. If
//! that run was filtered, the document is a subset and no amount of
//! re-querying recovers what was pruned. That is detectable —
//! `metrics.callables` counts the whole analysis while `callables` holds
//! what survived — so it is detected and said out loud rather than left
//! for the caller to notice by getting fewer results than they expected.
//!
//! Options that need facts the document does not carry are refused, not
//! ignored. A `--dead-code` replay would have no visibility records, no
//! attributes and no framework roots to reason from; silently emitting
//! an ordinary graph instead is the failure mode `--write-roots` used to
//! have, and it is worth refusing rather than repeating.

use std::path::Path;

use anyhow::{Context, Result, bail};

use cgg_core::audit::AuditEvent;
use cgg_core::graph::Graph;

use crate::outcome::{Emission, RunOutcome};
use crate::{RunOptions, query};

/// Options that need analysis-time facts a saved graph does not carry.
///
/// Each entry is `(flag, why)`. The `why` is in the error text because
/// "not supported with --from-graph" invites the reader to assume a
/// missing feature, when in every case here the information genuinely
/// is not in the document.
fn unsupported(opts: &RunOptions) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    if opts.dead_mode() {
        out.push((
            "--dead-code / --why-live / --write-roots",
            "needs per-callable visibility, attributes and framework roots, \
             which are analysis facts and are not in a saved graph",
        ));
    }
    if opts.report_unreferenced {
        out.push((
            "--report-unreferenced",
            "needs the root rules a saved graph does not carry",
        ));
    }
    if opts.since.is_some() {
        out.push((
            "--since",
            "resolves a git diff against callable spans in a working tree",
        ));
    }
    if !opts.lang.is_empty() {
        out.push((
            "--lang",
            "selects which files to parse; use --exclude-regex on the saved \
             graph instead",
        ));
    }
    if opts.include_external || opts.include_stdlib {
        out.push((
            "--include-external / --include-stdlib",
            "mints exit nodes from per-file unresolved-call buckets that are \
             not part of the graph document; re-run the analysis with them on",
        ));
    }
    if opts.dynamic_dispatch || opts.reference_edges {
        out.push((
            "--dynamic-dispatch / --reference-edges",
            "adds edges during resolution; re-run the analysis with them on",
        ));
    }
    if opts.no_entry_nodes {
        out.push((
            "--no-entry-nodes",
            "suppresses nodes at synthesis time; the saved graph already has \
             whatever the original run decided",
        ));
    }
    out
}

/// Load a saved graph and run the query/rollup half of the pipeline.
pub fn replay(opts: &RunOptions) -> Result<RunOutcome> {
    let path = opts
        .from_graph
        .as_deref()
        .expect("caller checked from_graph is set");

    if !opts.paths.is_empty() {
        bail!(
            "--from-graph replays a saved graph and takes no paths; drop the \
             path argument, or drop --from-graph to analyze source"
        );
    }
    let bad = unsupported(opts);
    if !bad.is_empty() {
        let detail = bad
            .iter()
            .map(|(flag, why)| format!("  {flag}\n    {why}"))
            .collect::<Vec<_>>()
            .join("\n");
        bail!("these options cannot be honoured on a replayed graph:\n{detail}");
    }

    let mut transcript: Vec<Emission> = Vec::new();
    let mut events: Vec<AuditEvent> = Vec::new();

    let graph = load(path, &mut transcript, &mut events)?;
    let metrics = graph.metrics.clone();

    let (graph, query_stats) =
        query::apply_query(&graph, &opts.filter, opts.hops, opts.max_paths)
            .map_err(|e| anyhow::anyhow!(e))?;
    if query_stats.paths_truncated {
        events.push(AuditEvent::PathsTruncated {
            max_paths: opts.max_paths,
            paths_emitted: query_stats.paths_emitted,
        });
        transcript.push(Emission::line(format!(
            "cgg: -n 0 stopped at --max-paths {} ({} path(s) kept) — the graph \
             omits paths that exist; raise --max-paths or narrow --filter\n",
            opts.max_paths, query_stats.paths_emitted,
        )));
    }
    let graph = query::apply_exclusions(
        &graph,
        &opts.exclude_partial,
        &opts.exclude_glob,
        &opts.exclude_regex,
    )
    .map_err(|e| anyhow::anyhow!(e))?;

    // Rollup levels below `file` key on a path *relative to the analysis
    // root*, and a replay was not given one — the invocation has no
    // paths at all. Recovering it from the document is not guesswork:
    // the deepest directory every analyzed file sits under is exactly
    // what the original run would have stripped. Without this, `dir:1`
    // groups an absolute-path graph under `/home` and returns one node.
    let roots = roots_from_graph(&graph);
    let graph = crate::apply_rollup(graph, opts, &roots, &mut transcript, &mut events)?;

    transcript.push(Emission::Graph);
    transcript.push(Emission::Audit);
    transcript.push(Emission::line(format!(
        "cgg: replayed {} — {} callables, {} edges\n",
        path.display(),
        graph.callables.len(),
        graph.edges.len(),
    )));

    Ok(RunOutcome {
        graph,
        transcript,
        events,
        metrics,
        framework_coverage: Default::default(),
        dead_code: None,
        dead_code_marked: 0,
        dead_code_threshold: opts.dead_code_confidence,
        cross_file_edges: 0,
        jobs: 0,
    })
}

/// Recover the analysis root from a loaded graph's own file paths.
///
/// The longest directory prefix shared by every real source file.
/// Sentinel records (`<external>`, `<framework-entry>`) are excluded —
/// they are not paths, and including them would drag the common prefix
/// to nothing.
///
/// Returns an empty list when there is no shared prefix to strip, which
/// is the correct input to `id_path`: it then leaves paths as they are.
fn roots_from_graph(graph: &Graph) -> Vec<crate::stable_ids::IdRoot> {
    let mut dirs = graph
        .files
        .values()
        .map(|f| f.path.as_path())
        .filter(|p| !p.to_string_lossy().starts_with('<'))
        .filter_map(|p| p.parent());

    let Some(first) = dirs.next() else {
        return Vec::new();
    };
    let mut common: Vec<std::ffi::OsString> = first
        .components()
        .map(|c| c.as_os_str().to_owned())
        .collect();
    for dir in dirs {
        let other: Vec<_> = dir.components().map(|c| c.as_os_str()).collect();
        let keep = common
            .iter()
            .zip(other.iter())
            .take_while(|(a, b)| a.as_os_str() == **b)
            .count();
        common.truncate(keep);
        if common.is_empty() {
            return Vec::new();
        }
    }
    let display: std::path::PathBuf = common.iter().collect();
    if display.as_os_str().is_empty() {
        return Vec::new();
    }
    vec![crate::stable_ids::IdRoot {
        display,
        prefix: String::new(),
    }]
}

/// Read and validate the document.
fn load(
    path: &Path,
    transcript: &mut Vec<Emission>,
    events: &mut Vec<AuditEvent>,
) -> Result<Graph> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading saved graph {}", path.display()))?;

    // Peek at the wrapper keys before deserialising. They are not on
    // `Graph` — see `cgg_format::json` — so this is the only place they
    // can be checked, and checking them is the whole reason they exist.
    let peek: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} is not valid JSON. --from-graph reads a document written by \
             `cgg ... -t json`",
            path.display()
        )
    })?;
    match peek.get("schema").and_then(|v| v.as_str()) {
        Some(cgg_format::json::GRAPH_SCHEMA) => {}
        Some(other) => bail!(
            "{} declares schema {other:?}, but this cgg reads {:?}",
            path.display(),
            cgg_format::json::GRAPH_SCHEMA
        ),
        // Written before the schema key existed, or by something else
        // entirely. Readable, but node ids are explicitly not comparable
        // across cgg versions, so an unlabelled document is a warning
        // rather than a silent success.
        None => transcript.push(Emission::always(format!(
            "warning: {} carries no `schema` key — it predates cgg {} or was \
             not written by cgg. Reading it anyway; node ids are not \
             comparable across versions.\n",
            path.display(),
            cgg_core::version::CGG_VERSION
        ))),
    }
    if let Some(v) = peek.get("cgg_version").and_then(|v| v.as_str())
        && v != cgg_core::version::CGG_VERSION
    {
        transcript.push(Emission::always(format!(
            "warning: {} was written by cgg {v}, this is cgg {}. Node ids are \
             not comparable across versions.\n",
            path.display(),
            cgg_core::version::CGG_VERSION
        )));
    }

    let graph: Graph = serde_json::from_slice(&bytes).with_context(|| {
        format!("{} is JSON but not a cgg graph document", path.display())
    })?;

    // A saved graph whose own metrics outnumber its contents was already
    // narrowed. Re-querying it can only narrow it further, and a caller
    // who does not know that reads a partial answer as a complete one.
    let analyzed = graph.metrics.callables;
    let present = graph.callables.len() as u64;
    let source_filtered = analyzed > present;
    if source_filtered {
        transcript.push(Emission::always(format!(
            "warning: {} holds {present} of the {analyzed} callable(s) its run \
             analyzed — it was already filtered. Anything pruned then cannot \
             come back now; re-run the analysis to widen it.\n",
            path.display()
        )));
    }
    events.push(AuditEvent::GraphReplayed {
        path: path.to_path_buf(),
        callables: present,
        edges: graph.edges.len() as u64,
        source_filtered,
    });
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunOptions;
    use std::path::PathBuf;

    fn saved(dir: &Path, graph: &Graph) -> PathBuf {
        let p = dir.join("g.json");
        let s = crate::emit::graph_to_string(graph, crate::OutputFormat::Json);
        std::fs::write(&p, s).unwrap();
        p
    }

    fn fixture() -> Graph {
        use cgg_core::graph::{CallEdge, CallableNode, FileRecord};
        use cgg_core::ids::{CallableId, FileId, ResolverId};
        let mut g = Graph::new();
        g.add_file(FileRecord {
            id: FileId::new(0),
            path: PathBuf::from("a.rs"),
            language: "rust".into(),
            blake3: "0".repeat(64),
            ..Default::default()
        });
        for i in 0..3u32 {
            g.add_callable(CallableNode {
                id: CallableId::new(i),
                qualified_name: format!("m::f{i}"),
                simple_name: format!("f{i}"),
                language: "rust".into(),
                file: FileId::new(0),
                ..Default::default()
            });
        }
        for i in 0..2u32 {
            g.add_edge(CallEdge {
                src: CallableId::new(i),
                dst: CallableId::new(i + 1),
                resolver: ResolverId::new("test"),
                ..Default::default()
            });
        }
        g
    }

    #[test]
    fn a_saved_graph_round_trips_and_can_be_filtered_again() {
        let tmp = tempfile::tempdir().unwrap();
        let p = saved(tmp.path(), &fixture());
        let out = replay(&RunOptions {
            from_graph: Some(p),
            filter: vec!["f1".into()],
            hops: 1,
            ..RunOptions::default()
        })
        .unwrap();
        // 1 hop around f1 is f0, f1, f2.
        assert_eq!(out.graph.callables.len(), 3);
        assert!(out.transcript.iter().any(|e| matches!(e, Emission::Graph)));
    }

    #[test]
    fn the_analysis_root_is_recovered_from_the_documents_own_paths() {
        // REGRESSION: a replay has no `--paths`, so nothing stripped the
        // analysis root and `dir:1` grouped an absolute-path graph under
        // `/home` — one node for the entire tree.
        use cgg_core::graph::{CallableNode, FileRecord};
        use cgg_core::ids::{CallableId, FileId};
        let mut g = Graph::new();
        for (i, p) in ["/srv/proj/a/one.rs", "/srv/proj/b/two.rs"]
            .iter()
            .enumerate()
        {
            g.add_file(FileRecord {
                id: FileId::new(i as u32),
                path: PathBuf::from(p),
                language: "rust".into(),
                ..Default::default()
            });
            g.add_callable(CallableNode {
                id: CallableId::new(i as u32),
                qualified_name: format!("f{i}"),
                language: "rust".into(),
                file: FileId::new(i as u32),
                ..Default::default()
            });
        }
        let tmp = tempfile::tempdir().unwrap();
        let p = saved(tmp.path(), &g);
        let out = replay(&RunOptions {
            from_graph: Some(p),
            rollup_by: Some(crate::rollup::RollupLevel::Dir(1)),
            ..RunOptions::default()
        })
        .unwrap();
        let mut names: Vec<&str> = out
            .graph
            .callables
            .values()
            .map(|c| c.qualified_name.as_str())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["a", "b"],
            "the shared /srv/proj prefix must be stripped"
        );
    }

    #[test]
    fn replay_composes_with_rollup() {
        let tmp = tempfile::tempdir().unwrap();
        let p = saved(tmp.path(), &fixture());
        let out = replay(&RunOptions {
            from_graph: Some(p),
            rollup_by: Some(crate::rollup::RollupLevel::File),
            ..RunOptions::default()
        })
        .unwrap();
        assert_eq!(out.graph.callables.len(), 1, "three callables, one file");
        assert!(out.graph.callables.values().all(|c| c.rollup.is_some()));
    }

    #[test]
    fn a_previously_filtered_document_warns_instead_of_quietly_under_reporting() {
        let tmp = tempfile::tempdir().unwrap();
        let mut g = fixture();
        // What `prune` leaves behind: whole-run metrics, subset contents.
        g.metrics.callables = 99;
        let p = saved(tmp.path(), &g);
        let out = replay(&RunOptions {
            from_graph: Some(p),
            ..RunOptions::default()
        })
        .unwrap();
        assert!(
            out.notices().any(|n| n.contains("already filtered")),
            "notices were: {:?}",
            out.notices().collect::<Vec<_>>()
        );
    }

    #[test]
    fn options_that_need_analysis_facts_are_refused_not_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let p = saved(tmp.path(), &fixture());
        let err = replay(&RunOptions {
            from_graph: Some(p.clone()),
            dead_code: true,
            ..RunOptions::default()
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("--dead-code"), "{err}");

        let err = replay(&RunOptions {
            from_graph: Some(p),
            include_external: true,
            ..RunOptions::default()
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("--include-external"), "{err}");
    }

    #[test]
    fn paths_and_from_graph_together_are_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let p = saved(tmp.path(), &fixture());
        let err = replay(&RunOptions {
            from_graph: Some(p),
            paths: vec![PathBuf::from("./src")],
            ..RunOptions::default()
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("takes no paths"), "{err}");
    }

    #[test]
    fn a_document_with_the_wrong_schema_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("g.json");
        std::fs::write(&p, r#"{"schema":"cgg.graph.v99","callables":{}}"#).unwrap();
        let err = replay(&RunOptions {
            from_graph: Some(p),
            ..RunOptions::default()
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("cgg.graph.v99"), "{err}");
    }

    #[test]
    fn a_document_with_no_schema_warns_but_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("g.json");
        // What a pre-schema `-t json` run wrote.
        let raw = serde_json::to_string(&fixture()).unwrap();
        std::fs::write(&p, raw).unwrap();
        let out = replay(&RunOptions {
            from_graph: Some(p),
            ..RunOptions::default()
        })
        .unwrap();
        assert_eq!(out.graph.callables.len(), 3);
        assert!(out.notices().any(|n| n.contains("no `schema` key")));
    }
}
