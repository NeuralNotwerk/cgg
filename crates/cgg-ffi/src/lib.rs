//! C ABI for cgg. One shared library, every language that can call C.
//!
//! A translation layer over [`cgg::analyze`] with no analysis logic, the
//! same contract `crates/cgg-py` holds. The difference is the boundary:
//! Python gets real objects, C gets strings, because a C ABI that mirrors
//! the object graph would need a function per field and would break every
//! time one is added.
//!
//! # The shape
//!
//! Options go in as a JSON document and results come out as strings. That
//! is what lets one ABI serve C, .NET, Java, Go, Ruby and anything else
//! with an FFI: adding a CLI flag adds a JSON key, not an entry point, so
//! **the ABI does not change when cgg gains a feature**. Every wrapper
//! language keeps working without being rebuilt.
//!
//! It is affordable because rendering is nearly free next to analysis —
//! measured on cgg's own tree, analysis is 137.7 ms while `to_json()` is
//! 6.5 ms (4.7%) and `to_mermaid()` is 1.3 ms (0.9%).
//!
//! Analysis returns an opaque handle rather than a rendered string, so a
//! caller that wants mermaid *and* the metrics *and* the JSON pays for
//! one analysis, not three.
//!
//! # Rules for every caller
//!
//! * Every `char*` this library returns must be released with
//!   [`cgg_string_free`], and every `cgg_graph*` with [`cgg_graph_free`].
//!   They are allocated by this library's allocator; `free(3)` is wrong.
//! * A returned pointer is NUL-terminated UTF-8.
//! * `NULL` in, error out — no input pointer is dereferenced unchecked.
//! * Every entry point catches Rust panics. A panic unwinding into C is
//!   undefined behaviour, so each one is a caught error instead.
//! * `cgg_graph` is immutable once returned, and safe to share and render
//!   from several threads at once.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use cgg_format::OutputFormat;

/// See `crates/cgg/src/main.rs` — extraction is allocation-heavy and the
/// system allocator serialises under it.
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Success.
pub const CGG_OK: i32 = 0;
/// A pointer argument was NULL, or a string was not valid UTF-8.
pub const CGG_ERR_INVALID_ARG: i32 = 1;
/// The options JSON did not parse, or named a key cgg does not have.
pub const CGG_ERR_BAD_OPTIONS: i32 = 2;
/// The analysis itself failed — a missing path, an unparsable filter.
pub const CGG_ERR_ANALYSIS: i32 = 3;
/// A panic was caught at the boundary. A bug in cgg; please report it.
pub const CGG_ERR_PANIC: i32 = 4;

/// An analyzed call graph. Opaque to C; freed with [`cgg_graph_free`].
///
/// Holds the whole [`cgg::RunOutcome`] rather than just the graph so that
/// metrics and notices stay available without a second analysis.
// `cgg_graph`, not `CggGraph`: this identifier is part of the C API and
// appears verbatim in cgg.h, where snake_case is the convention every
// caller expects. Renaming it to satisfy Rust style would make the header
// and the symbol disagree.
#[allow(non_camel_case_types)]
pub struct cgg_graph {
    outcome: cgg::RunOutcome,
}

/// Hand a `String` to C as an owned NUL-terminated buffer.
///
/// Returns `NULL` if the string contains an interior NUL, which C has no
/// way to represent. Formatter output cannot, but an error message built
/// from a user-supplied path could.
fn into_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Write `msg` through an out-parameter, if the caller supplied one.
///
/// # Safety
/// `slot` must be null or a valid, writable `*mut *mut c_char`.
unsafe fn set_err(slot: *mut *mut c_char, msg: String) {
    if !slot.is_null() {
        unsafe { *slot = into_c_string(msg) };
    }
}

/// Borrow a C string as `&str`.
///
/// # Safety
/// `p` must be null or point to a NUL-terminated buffer that outlives the
/// borrow.
unsafe fn as_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// Run every entry point's body with panics caught.
///
/// A Rust panic crossing into C is undefined behaviour, and cgg has
/// `panic = "unwind"`, so this is not optional.
fn guard<T>(on_panic: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(on_panic)
}

/// The cgg version, as a static NUL-terminated string.
///
/// Never freed — it is a literal, not an allocation.
#[unsafe(no_mangle)]
pub extern "C" fn cgg_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Analyze a source tree.
///
/// `options_json` is a JSON object of [`cgg::RunOptions`] fields; `NULL`
/// or `"{}"` means defaults, and `paths` is the only one that must be
/// set. Unknown keys are an error rather than being ignored, so a typo is
/// reported instead of silently doing nothing.
///
/// ```c
/// char *err = NULL;
/// cgg_graph *g = cgg_analyze("{\"paths\":[\"./src\"]}", &err);
/// if (!g) { fputs(err, stderr); cgg_string_free(err); return 1; }
/// ```
///
/// Returns `NULL` on failure, setting `*err` to an owned message when
/// `err` is non-NULL. Free the result with [`cgg_graph_free`].
///
/// # Safety
/// `options_json` must be null or NUL-terminated; `err` must be null or
/// writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgg_analyze(
    options_json: *const c_char,
    err: *mut *mut c_char,
) -> *mut cgg_graph {
    if !err.is_null() {
        unsafe { *err = ptr::null_mut() };
    }
    guard(ptr::null_mut(), || {
        let json = if options_json.is_null() {
            "{}"
        } else {
            match unsafe { as_str(options_json) } {
                Some(s) => s,
                None => {
                    unsafe {
                        set_err(err, "options_json is not valid UTF-8".into());
                    }
                    return ptr::null_mut();
                }
            }
        };
        let json = if json.trim().is_empty() { "{}" } else { json };

        let opts: cgg::RunOptions = match serde_json::from_str(json) {
            Ok(o) => o,
            Err(e) => {
                unsafe { set_err(err, format!("invalid options JSON: {e}")) };
                return ptr::null_mut();
            }
        };

        match cgg::analyze(&opts) {
            // `{:#}` so the whole anyhow context chain crosses, not just
            // the outermost message.
            Err(e) => {
                unsafe { set_err(err, format!("{e:#}")) };
                ptr::null_mut()
            }
            Ok(outcome) => Box::into_raw(Box::new(cgg_graph { outcome })),
        }
    })
}

/// Render an analyzed graph.
///
/// `format` is `"mermaid"`, `"json"`, `"dot"` or `"graphml"`. Rendering
/// does not consume the handle, so all four are available from one
/// analysis.
///
/// Returns `NULL` on failure. Free the result with [`cgg_string_free`].
///
/// # Safety
/// `graph` must come from [`cgg_analyze`] and not yet be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgg_graph_render(
    graph: *const cgg_graph,
    format: *const c_char,
    err: *mut *mut c_char,
) -> *mut c_char {
    if !err.is_null() {
        unsafe { *err = ptr::null_mut() };
    }
    guard(ptr::null_mut(), || {
        let Some(g) = (unsafe { graph.as_ref() }) else {
            unsafe { set_err(err, "graph is NULL".into()) };
            return ptr::null_mut();
        };
        let name = match unsafe { as_str(format) } {
            Some(s) => s,
            None => {
                unsafe { set_err(err, "format is NULL or not valid UTF-8".into()) };
                return ptr::null_mut();
            }
        };
        // Parsed here rather than as an int, so the C header cannot drift
        // out of step with the Rust enum's discriminants.
        let fmt = match name {
            "mermaid" => OutputFormat::Mermaid,
            "json" => OutputFormat::Json,
            "dot" => OutputFormat::Dot,
            "graphml" => OutputFormat::Graphml,
            other => {
                unsafe {
                    set_err(
                        err,
                        format!(
                            "unknown format {other:?} — expected one of \
                             mermaid, json, dot, graphml"
                        ),
                    )
                };
                return ptr::null_mut();
            }
        };
        into_c_string(cgg::emit::graph_to_string(&g.outcome.graph, fmt))
    })
}

/// Everything about the run that is not the graph, as one JSON object:
/// `callables`, `edges`, `cross_file_edges`, `jobs`, `dead_code_marked`,
/// `metrics` and `notices`.
///
/// One call rather than an accessor per field, for the same reason the
/// options arrive as JSON: a new counter is a new key, not a new symbol,
/// so no wrapper has to be rebuilt to see it.
///
/// Returns `NULL` on failure. Free the result with [`cgg_string_free`].
///
/// # Safety
/// `graph` must come from [`cgg_analyze`] and not yet be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgg_graph_meta(
    graph: *const cgg_graph,
    err: *mut *mut c_char,
) -> *mut c_char {
    if !err.is_null() {
        unsafe { *err = ptr::null_mut() };
    }
    guard(ptr::null_mut(), || {
        let Some(g) = (unsafe { graph.as_ref() }) else {
            unsafe { set_err(err, "graph is NULL".into()) };
            return ptr::null_mut();
        };
        let o = &g.outcome;
        let meta = serde_json::json!({
            "callables": o.graph.callables.len(),
            "edges": o.graph.edges.len(),
            "cross_file_edges": o.cross_file_edges,
            "jobs": o.jobs,
            "dead_code_marked": o.dead_code_marked,
            "metrics": o.metrics,
            "notices": o.notices().collect::<Vec<_>>(),
        });
        match serde_json::to_string(&meta) {
            Ok(s) => into_c_string(s),
            Err(e) => {
                unsafe { set_err(err, format!("serialising metadata: {e}")) };
                ptr::null_mut()
            }
        }
    })
}

/// Number of callables in the graph, or `-1` if `graph` is NULL.
///
/// A convenience for the commonest probe, so a caller does not have to
/// parse [`cgg_graph_meta`] to answer "did this find anything".
///
/// # Safety
/// `graph` must come from [`cgg_analyze`] and not yet be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgg_graph_callable_count(graph: *const cgg_graph) -> i64 {
    guard(-1, || match unsafe { graph.as_ref() } {
        Some(g) => g.outcome.graph.callables.len() as i64,
        None => -1,
    })
}

/// Release a graph returned by [`cgg_analyze`]. `NULL` is a no-op.
///
/// # Safety
/// `graph` must come from [`cgg_analyze`] and must not be used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgg_graph_free(graph: *mut cgg_graph) {
    if graph.is_null() {
        return;
    }
    // Guarded like every other entry point: a `Drop` impl deep in the
    // outcome could panic, and unwinding into C is UB even on a free.
    guard((), || {
        drop(unsafe { Box::from_raw(graph) });
    });
}

/// Release a string returned by this library. `NULL` is a no-op.
///
/// # Safety
/// `s` must come from this library and must not be used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgg_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    guard((), || {
        drop(unsafe { CString::from_raw(s) });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    /// Drive the ABI the way C does, so these exercise the real path.
    fn analyze(json: &str) -> (*mut cgg_graph, Option<String>) {
        let c = CString::new(json).unwrap();
        let mut err: *mut c_char = ptr::null_mut();
        let g = unsafe { cgg_analyze(c.as_ptr(), &mut err) };
        let msg = if err.is_null() {
            None
        } else {
            let s = unsafe { CStr::from_ptr(err) }.to_str().unwrap().to_string();
            unsafe { cgg_string_free(err) };
            Some(s)
        };
        (g, msg)
    }

    fn take(p: *mut c_char) -> String {
        let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        unsafe { cgg_string_free(p) };
        s
    }

    #[test]
    fn analyze_and_render_every_format() {
        let (g, err) = analyze(r#"{"paths":["../cgg-walk"]}"#);
        assert!(!g.is_null(), "analyze failed: {err:?}");
        assert!(unsafe { cgg_graph_callable_count(g) } > 0);

        for (name, needle) in [
            ("mermaid", "flowchart LR"),
            ("json", "\"callables\""),
            ("dot", "digraph"),
            ("graphml", "<graphml"),
        ] {
            let f = CString::new(name).unwrap();
            let mut err = ptr::null_mut();
            let out = unsafe { cgg_graph_render(g, f.as_ptr(), &mut err) };
            assert!(!out.is_null(), "{name} render failed");
            let s = take(out);
            assert!(s.contains(needle), "{name} output missing {needle:?}");
        }
        unsafe { cgg_graph_free(g) };
    }

    /// The whole point of the JSON boundary: options actually apply.
    #[test]
    fn options_change_the_graph() {
        let (a, _) = analyze(r#"{"paths":["../cgg-walk"]}"#);
        let (b, _) = analyze(r#"{"paths":["../cgg-walk"],"include_stdlib":true}"#);
        assert!(!a.is_null() && !b.is_null());
        let na = unsafe { cgg_graph_callable_count(a) };
        let nb = unsafe { cgg_graph_callable_count(b) };
        assert!(nb > na, "include_stdlib added no exit nodes: {na} -> {nb}");
        unsafe { cgg_graph_free(a) };
        unsafe { cgg_graph_free(b) };
    }

    /// A typo must be reported, not ignored — the failure mode this ABI
    /// is most exposed to, since C callers hand-write the JSON.
    #[test]
    fn unknown_option_key_is_an_error() {
        let (g, err) = analyze(r#"{"paths":["../cgg-walk"],"hopz":2}"#);
        assert!(g.is_null());
        let msg = err.expect("expected an error message");
        assert!(
            msg.contains("hopz"),
            "message should name the bad key: {msg}"
        );
    }

    #[test]
    fn errors_are_reported_not_crashed() {
        let (g, err) = analyze(r#"{"paths":["/definitely/not/here"]}"#);
        assert!(g.is_null());
        assert!(err.unwrap().contains("does not exist"));

        let (g, err) = analyze("not json at all");
        assert!(g.is_null());
        assert!(err.unwrap().contains("invalid options JSON"));
    }

    /// NULL must never be dereferenced.
    #[test]
    fn null_arguments_are_handled() {
        assert_eq!(unsafe { cgg_graph_callable_count(ptr::null()) }, -1);
        let f = CString::new("mermaid").unwrap();
        let mut err = ptr::null_mut();
        assert!(unsafe { cgg_graph_render(ptr::null(), f.as_ptr(), &mut err) }.is_null());
        assert!(!err.is_null());
        unsafe { cgg_string_free(err) };
        // Frees of NULL are no-ops, not faults.
        unsafe { cgg_graph_free(ptr::null_mut()) };
        unsafe { cgg_string_free(ptr::null_mut()) };
        // A NULL options blob means defaults, which have no paths.
        let mut err = ptr::null_mut();
        assert!(unsafe { cgg_analyze(ptr::null(), &mut err) }.is_null());
        assert!(!err.is_null());
        unsafe { cgg_string_free(err) };
    }

    #[test]
    fn meta_carries_counts_and_notices() {
        let (g, _) = analyze(r#"{"paths":["../cgg-walk"]}"#);
        assert!(!g.is_null());
        let mut err = ptr::null_mut();
        let m = take(unsafe { cgg_graph_meta(g, &mut err) });
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert!(v["callables"].as_u64().unwrap() > 0);
        assert!(v["jobs"].as_u64().unwrap() >= 1);
        assert!(v["metrics"]["files_analyzed"].as_u64().unwrap() >= 1);
        assert!(
            v["notices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n.as_str().unwrap().contains("callables")),
            "run summary should be among the notices"
        );
        unsafe { cgg_graph_free(g) };
    }

    /// `jobs` is observable, so it is testable — the same reason
    /// `RunOutcome::jobs` exists.
    #[test]
    fn jobs_is_honoured_through_the_abi() {
        for want in [1u64, 3] {
            let (g, _) =
                analyze(&format!(r#"{{"paths":["../cgg-walk"],"jobs":{want}}}"#));
            assert!(!g.is_null());
            let mut err = ptr::null_mut();
            let m = take(unsafe { cgg_graph_meta(g, &mut err) });
            let v: serde_json::Value = serde_json::from_str(&m).unwrap();
            assert_eq!(v["jobs"].as_u64().unwrap(), want);
            unsafe { cgg_graph_free(g) };
        }
    }

    /// Handles are immutable and shareable; the CLI's own guarantee is
    /// that output does not depend on how the work was scheduled.
    #[test]
    fn graphs_render_from_several_threads() {
        let (g, _) = analyze(r#"{"paths":["../cgg-walk"]}"#);
        assert!(!g.is_null());
        let addr = g as usize;
        let hs: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    let f = CString::new("mermaid").unwrap();
                    let mut err = ptr::null_mut();
                    let p = unsafe {
                        cgg_graph_render(addr as *const cgg_graph, f.as_ptr(), &mut err)
                    };
                    assert!(!p.is_null());
                    take(p)
                })
            })
            .collect();
        let outs: Vec<String> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(outs.windows(2).all(|w| w[0] == w[1]));
        unsafe { cgg_graph_free(g) };
    }
}
