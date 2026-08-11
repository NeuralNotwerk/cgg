/*
 * cgg.h — C ABI for the cgg call-graph generator.
 *
 * Offline, deterministic call graphs for 44 languages. One shared library
 * serves every language with an FFI, so a binding is source rather than
 * another native artifact per target.
 *
 * Link against libcgg.so / libcgg.dylib / cgg.dll (cdylib), or libcgg.a
 * (staticlib) to keep your program a single binary.
 *
 * THE SHAPE
 *   Options go in as a JSON document; results come out as strings. Adding
 *   a cgg feature adds a JSON key, not an entry point, so this header does
 *   not change when cgg gains a flag and your binding keeps working.
 *
 *   cgg_analyze() returns an opaque handle, not a rendered string, so
 *   asking for mermaid AND json AND the metrics costs one analysis.
 *
 * MEMORY
 *   Every char* returned here is released with cgg_string_free(), and
 *   every cgg_graph* with cgg_graph_free(). They come from this library's
 *   allocator — calling free(3) on them is wrong. Passing NULL to either
 *   is a no-op.
 *
 * ERRORS
 *   Functions returning a pointer return NULL on failure and set *err to
 *   an owned message when err is non-NULL (free it with
 *   cgg_string_free). Pass NULL for err if you do not want the detail.
 *   Rust panics are caught at the boundary and reported, never unwound
 *   into your program.
 *
 * THREADS
 *   cgg_analyze() may be called concurrently. A cgg_graph is immutable
 *   once returned and may be rendered from several threads at once.
 *
 * Licensed Apache-2.0 OR MIT. https://github.com/NeuralNotwerk/cgg
 */

#ifndef CGG_H
#define CGG_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Status codes. Reserved for future use by functions that return int32_t;
 * the pointer-returning functions below signal failure with NULL. */
#define CGG_OK                0
#define CGG_ERR_INVALID_ARG   1
#define CGG_ERR_BAD_OPTIONS   2
#define CGG_ERR_ANALYSIS      3
#define CGG_ERR_PANIC         4

/* An analyzed call graph. Opaque. */
typedef struct cgg_graph cgg_graph;

/* Library version, e.g. "0.6.0". Static storage — do not free. */
const char *cgg_version(void);

/*
 * Analyze a source tree.
 *
 * options_json is a JSON object; NULL or "{}" means defaults. "paths" is
 * the only required key. Unknown keys are an ERROR, not ignored, so a
 * typo is reported rather than silently doing nothing.
 *
 *   {
 *     "paths":             ["./src"],          // required
 *     "filter":            ["^auth::"],        // regex on qualified names
 *     "hops":              2,                  // -1 = whole graph, 0 = full paths
 *     "max_paths":         1000,
 *     "lang":              ["rust", "python"],
 *     "jobs":              0,                  // 0 = half the physical cores
 *     "exclude_partial":   [], "exclude_glob": [], "exclude_regex": [],
 *     "include_external":  false, "include_stdlib":  false,
 *     "dynamic_dispatch":  false, "reference_edges": false,
 *     "no_entry_nodes":    false, "include_tests":   false,
 *     "dead_code":         false, "dead_code_confidence": "high",
 *     "since":             "main..HEAD",
 *     "ignore_file": null, "roots": null,
 *     "ignore_names": [], "ignore_attributes": []
 *   }
 *
 * Returns NULL on failure. Free the result with cgg_graph_free().
 */
cgg_graph *cgg_analyze(const char *options_json, char **err);

/*
 * Render a graph. format is "mermaid", "json", "dot" or "graphml".
 * Does not consume the handle — all four are available from one analysis.
 * Returns NULL on failure. Free the result with cgg_string_free().
 */
char *cgg_graph_render(const cgg_graph *graph, const char *format, char **err);

/*
 * Everything about the run that is not the graph, as one JSON object:
 * callables, edges, cross_file_edges, jobs, dead_code_marked, metrics,
 * notices. One call rather than an accessor per field, so a new counter
 * does not need a new symbol.
 * Returns NULL on failure. Free the result with cgg_string_free().
 */
char *cgg_graph_meta(const cgg_graph *graph, char **err);

/* Callables in the graph, or -1 if graph is NULL. */
int64_t cgg_graph_callable_count(const cgg_graph *graph);

/* Release a graph from cgg_analyze(). NULL is a no-op. */
void cgg_graph_free(cgg_graph *graph);

/* Release a string from this library. NULL is a no-op. */
void cgg_string_free(char *s);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* CGG_H */
