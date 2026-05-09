//! Stack-graphs-backed resolver for Python, JavaScript, TypeScript, Java.
//!
//! This module wraps `tree-sitter-stack-graphs-{python,javascript,typescript,java}`
//! behind a single entry point. High-level flow:
//!
//! 1. For each file belonging to a supported language, build a
//!    per-file stack graph using the language's `.tsg` rules.
//! 2. Run `find_minimal_partial_path_set_in_file` per file to
//!    populate the path database.
//! 3. For each reference node, run
//!    `ForwardPartialPathStitcher::find_all_complete_partial_paths`
//!    to discover resolved definitions.
//! 4. Map stack-graph nodes back to our `CallableId`s by byte range
//!    (smallest-enclosing for refs; exact-or-containing for defs) and
//!    emit a `CallEdge` with `resolver="stack-graphs:<lang>"` and
//!    `confidence=High` for single-candidate resolution, `Low` for
//!    ambiguous.

use std::collections::HashMap;

use cgg_core::{
    audit::AuditUnresolvedCall,
    graph::{CallEdge, Confidence, Graph, Via},
    ids::{CallableId, FileId, ResolverId},
    FileFacts,
};
use stack_graphs::{
    arena::Handle,
    graph::{File as SgFile, Node, StackGraph},
    partial::PartialPaths,
    stitching::{
        Database, DatabaseCandidates, ForwardPartialPathStitcher,
        StitcherConfig,
    },
    NoCancellation,
};
use tree_sitter_stack_graphs::{
    loader::LanguageConfiguration,
    NoCancellation as SgNoCancellation, Variables,
};

/// Output of a full resolution pass.
#[derive(Debug, Default)]
pub struct ResolveOutput {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<AuditUnresolvedCall>,
}

/// Inputs — we need the parsed source for every file we want stack
/// graphs to analyze.
#[derive(Debug)]
pub struct FileInput<'a> {
    pub file: FileId,
    pub language: &'a str,
    pub source: &'a [u8],
}

/// Run stack-graphs resolution across every file in `inputs` whose
/// language is supported. Edges are emitted for each `(reference,
/// resolved_definition)` pair that we can map back to our `CallableId`s.
pub fn resolve(
    graph: &Graph,
    facts: &[FileFacts],
    inputs: &[FileInput<'_>],
) -> ResolveOutput {
    let mut out = ResolveOutput::default();

    // Group inputs by language and keep only the four stack-graphs v1 languages.
    let mut by_lang: HashMap<&str, Vec<&FileInput<'_>>> = HashMap::new();
    for inp in inputs {
        if is_supported_language(inp.language) {
            by_lang.entry(inp.language).or_default().push(inp);
        }
    }

    // Build one stack graph + path database per language.
    for (lang, inputs) in by_lang {
        match resolve_language(lang, &inputs, graph, facts) {
            Ok(mut partial) => {
                out.edges.append(&mut partial.edges);
                out.unresolved.append(&mut partial.unresolved);
            }
            Err(e) => {
                tracing::warn!(lang, error = %e, "stack-graphs resolve failed");
            }
        }
    }

    out
}

fn is_supported_language(lang: &str) -> bool {
    matches!(lang, "python" | "javascript" | "typescript" | "java")
}

fn resolve_language(
    lang: &str,
    inputs: &[&FileInput<'_>],
    graph: &Graph,
    facts: &[FileFacts],
) -> anyhow::Result<ResolveOutput> {
    let cancel = SgNoCancellation;
    let lang_cfg = language_configuration(lang, &cancel)?;
    let mut stack_graph = StackGraph::new();
    let mut partials = PartialPaths::new();

    // Merge the language's prebuilt builtins graph so standard symbols
    // (like Python's `print`, `range`, …) are resolvable.
    let _ = stack_graph.add_from_graph(&lang_cfg.builtins);

    // Map FileId -> SG file handle for later span->CallableId lookups.
    let mut sg_to_cgg: HashMap<Handle<SgFile>, FileId> = HashMap::new();

    // Facts index: FileId -> facts (for enclosing-callable / def lookup).
    let facts_by_file: HashMap<FileId, &FileFacts> =
        facts.iter().map(|f| (f.file, f)).collect();

    for inp in inputs {
        // Unique SG path key per file — use the numeric FileId for safety.
        let key = format!("cgg:{}:{}", inp.language, inp.file.as_u32());
        let handle = stack_graph.get_or_create_file(&key);
        sg_to_cgg.insert(handle, inp.file);

        let src = std::str::from_utf8(inp.source).unwrap_or("");
        let globals = Variables::new();
        let build_res = lang_cfg.sgl.build_stack_graph_into(
            &mut stack_graph,
            handle,
            src,
            &globals,
            &cancel,
        );
        if let Err(e) = build_res {
            tracing::debug!(
                lang,
                file = key.as_str(),
                error = %format!("{:?}", e),
                "stack graph build failed; skipping file"
            );
        }
    }

    // Populate a path database from each file's minimal partial path set.
    let mut db = Database::new();
    let file_handles: Vec<Handle<SgFile>> = stack_graph.iter_files().collect();
    let mut total_paths = 0usize;
    for file_handle in &file_handles {
        let mut per_file = 0usize;
        let _ = ForwardPartialPathStitcher::find_minimal_partial_path_set_in_file(
            &stack_graph,
            &mut partials,
            *file_handle,
            StitcherConfig::default().with_detect_similar_paths(true),
            &NoCancellation,
            |g, ps, p| {
                per_file += 1;
                db.add_partial_path(g, ps, p.clone());
            },
        );
        total_paths += per_file;
        tracing::debug!(
            lang,
            file = stack_graph[*file_handle].name(),
            paths = per_file,
            "stack-graphs: added partial paths",
        );
    }
    tracing::debug!(lang, total_paths, "stack-graphs: per-language path totals");

    // Dump a summary of the first few DB paths for debugging.
    let mut path_summary: Vec<String> = Vec::new();
    for ph in db.iter_partial_paths().take(80) {
        let p = &db[ph];
        let start_file = stack_graph[p.start_node]
            .file()
            .map(|f| stack_graph[f].name().to_string())
            .unwrap_or_else(|| "ROOT".into());
        let end_file = stack_graph[p.end_node]
            .file()
            .map(|f| stack_graph[f].name().to_string())
            .unwrap_or_else(|| "ROOT".into());
        path_summary.push(format!(
            "{start_file}(def={},ref={}) -> {end_file}(def={},ref={})",
            stack_graph[p.start_node].is_definition(),
            stack_graph[p.start_node].is_reference(),
            stack_graph[p.end_node].is_definition(),
            stack_graph[p.end_node].is_reference(),
        ));
    }
    tracing::debug!(lang, paths = ?path_summary, "stack-graphs: sample paths");

    // Build a per-file index of SG reference nodes by byte offset,
    // so we can find the SG node that corresponds to each call-site
    // RefRecord produced by the tree-sitter extractor.
    let mut refs_by_file: HashMap<FileId, Vec<(Handle<Node>, u32)>> = HashMap::new();
    for node in stack_graph.iter_nodes() {
        if !stack_graph[node].is_reference() {
            continue;
        }
        let Some((file, byte, _)) = span_for_node(&stack_graph, node) else {
            continue;
        };
        let Some(&cgg_file) = sg_to_cgg.get(&file) else {
            continue;
        };
        refs_by_file.entry(cgg_file).or_default().push((node, byte));
    }

    let resolver_id = ResolverId::new(format!("stack-graphs:{lang}"));
    let mut out = ResolveOutput::default();

    // Drive resolution from our tree-sitter call-site records, not
    // from every SG reference node. This filters out identifier uses
    // at definition sites, import names, etc. — the things that
    // aren't actual calls.
    for inp in inputs {
        let Some(facts) = facts_by_file.get(&inp.file) else {
            continue;
        };
        let empty = Vec::new();
        let sg_refs = refs_by_file.get(&inp.file).unwrap_or(&empty);

        for r in &facts.references {
            // Collect SG reference nodes at EXACTLY this call site's
            // byte position. Each call-expression creates one or two
            // reference nodes (the `push_symbol` for the callee and
            // an optional scope lookup); matching more broadly picks
            // up neighboring refs for other identifiers on the same
            // line and leads to cross-contamination.
            let matching: Vec<Handle<Node>> = sg_refs
                .iter()
                .filter(|(_, b)| *b == r.site_byte)
                .map(|(n, _)| *n)
                .collect();

            if matching.is_empty() {
                continue;
            }

            let src_cid = enclosing_callable(facts, r.site_byte, graph);

            let mut target_cids: Vec<CallableId> = Vec::new();
            for sg_ref in &matching {
                let _ = ForwardPartialPathStitcher::find_all_complete_partial_paths(
                    &mut DatabaseCandidates::new(
                        &stack_graph,
                        &mut partials,
                        &mut db,
                    ),
                    vec![*sg_ref],
                    StitcherConfig::default().with_detect_similar_paths(true),
                    &NoCancellation,
                    |g, _ps, path| {
                        if let Some((def_file_handle, def_byte, _)) =
                            span_for_node(g, path.end_node)
                        {
                            tracing::debug!(
                                ref_name = %r.name,
                                end_file = g[def_file_handle].name(),
                                end_byte = def_byte,
                                end_is_def = g[path.end_node].is_definition(),
                                end_is_ref = g[path.end_node].is_reference(),
                                "stack-graphs: resolved path endpoint",
                            );
                            if !g[path.end_node].is_definition() {
                                return;
                            }
                            if let Some(cid) = callable_at(
                                &sg_to_cgg,
                                &facts_by_file,
                                graph,
                                def_file_handle,
                                def_byte,
                            ) {
                                if !target_cids.contains(&cid) {
                                    target_cids.push(cid);
                                }
                            }
                        }
                    },
                );
            }

            match target_cids.as_slice() {
                [] => {
                    out.unresolved.push(AuditUnresolvedCall {
                        src: src_cid,
                        file: inp.file,
                        site_line: r.site_line,
                        site_byte: r.site_byte,
                        name: r.name.clone(),
                        reason: "stack-graphs:no-path".into(),
                    });
                }
                [target] => {
                    if let Some(src) = src_cid {
                        out.edges.push(CallEdge {
                            src,
                            dst: *target,
                            site_line: r.site_line,
                            site_byte: r.site_byte,
                            confidence: Confidence::High,
                            via: Via::Direct,
                            resolver: resolver_id.clone(),
                        });
                    } else {
                        out.unresolved.push(AuditUnresolvedCall {
                            src: None,
                            file: inp.file,
                            site_line: r.site_line,
                            site_byte: r.site_byte,
                            name: r.name.clone(),
                            reason: "stack-graphs:no-enclosing-callable".into(),
                        });
                    }
                }
                many => {
                    if let Some(src) = src_cid {
                        for t in many {
                            out.edges.push(CallEdge {
                                src,
                                dst: *t,
                                site_line: r.site_line,
                                site_byte: r.site_byte,
                                confidence: Confidence::Low,
                                via: Via::Direct,
                                resolver: resolver_id.clone(),
                            });
                        }
                    } else {
                        out.unresolved.push(AuditUnresolvedCall {
                            src: None,
                            file: inp.file,
                            site_line: r.site_line,
                            site_byte: r.site_byte,
                            name: r.name.clone(),
                            reason: "stack-graphs:ambiguous".into(),
                        });
                    }
                }
            }
        }
    }

    Ok(out)
}

/// Return (file_handle, start_byte, start_line_1based) for a graph node.
fn span_for_node(
    graph: &StackGraph,
    node: Handle<Node>,
) -> Option<(Handle<SgFile>, u32, u32)> {
    let file = graph[node].file()?;
    let span = graph.source_info(node)?.span.clone();
    let start_byte = span.start.containing_line.start as u32;
    let start_line = (span.start.line as u32) + 1;
    // Use the column start within the line to get exact byte offset.
    let precise_byte = (span.start.containing_line.start + span.start.column.utf8_offset) as u32;
    Some((file, precise_byte.max(start_byte), start_line))
}

fn enclosing_callable(
    facts: &FileFacts,
    byte: u32,
    graph: &Graph,
) -> Option<CallableId> {
    // Smallest enclosing def in this file, then look up its CallableId.
    let mut best: Option<(usize, u32)> = None;
    for (i, d) in facts.definitions.iter().enumerate() {
        if d.start_byte <= byte && byte < d.end_byte {
            let span = d.end_byte - d.start_byte;
            match best {
                None => best = Some((i, span)),
                Some((_, bspan)) if span < bspan => best = Some((i, span)),
                _ => {}
            }
        }
    }
    let (idx, _) = best?;
    let d = &facts.definitions[idx];
    // Find the CallableId in graph.callables by (file, byte range).
    graph
        .callables
        .values()
        .find(|c| {
            c.file == facts.file
                && c.start_byte == d.start_byte
                && c.end_byte == d.end_byte
        })
        .map(|c| c.id)
}

fn callable_at(
    sg_to_cgg: &HashMap<Handle<SgFile>, FileId>,
    facts_by_file: &HashMap<FileId, &FileFacts>,
    graph: &Graph,
    file_handle: Handle<SgFile>,
    byte: u32,
) -> Option<CallableId> {
    let cgg_file = *sg_to_cgg.get(&file_handle)?;
    let facts = facts_by_file.get(&cgg_file)?;
    let mut best: Option<(&cgg_core::DefRecord, u32)> = None;
    for d in &facts.definitions {
        if d.start_byte <= byte && byte < d.end_byte {
            let span = d.end_byte - d.start_byte;
            match best {
                None => best = Some((d, span)),
                Some((_, b)) if span < b => best = Some((d, span)),
                _ => {}
            }
        }
    }
    let (d, _) = best?;
    graph
        .callables
        .values()
        .find(|c| {
            c.file == cgg_file
                && c.start_byte == d.start_byte
                && c.end_byte == d.end_byte
        })
        .map(|c| c.id)
}

fn language_configuration(
    lang: &str,
    cancel: &SgNoCancellation,
) -> anyhow::Result<LanguageConfiguration> {
    Ok(match lang {
        "python" => tree_sitter_stack_graphs_python::try_language_configuration(cancel)
            .map_err(|e| anyhow::anyhow!("python language-configuration: {e}"))?,
        "javascript" => {
            tree_sitter_stack_graphs_javascript::try_language_configuration(cancel)
                .map_err(|e| anyhow::anyhow!("javascript language-configuration: {e}"))?
        }
        "typescript" => tree_sitter_stack_graphs_typescript::try_language_configuration_typescript(cancel)
            .map_err(|e| anyhow::anyhow!("typescript language-configuration: {e}"))?,
        "java" => tree_sitter_stack_graphs_java::try_language_configuration(cancel)
            .map_err(|e| anyhow::anyhow!("java language-configuration: {e}"))?,
        other => return Err(anyhow::anyhow!("unsupported stack-graphs language: {other}")),
    })
}
