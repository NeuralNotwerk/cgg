# cgg-ffi — the C ABI

One shared library, every language that can call C.

```c
#include "cgg.h"

char *err = NULL;
cgg_graph *g = cgg_analyze("{\"paths\":[\"./src\"]}", &err);
if (!g) { fputs(err, stderr); cgg_string_free(err); return 1; }

char *mermaid = cgg_graph_render(g, "mermaid", &err);
puts(mermaid);

cgg_string_free(mermaid);
cgg_graph_free(g);
```

A translation layer over `cgg::analyze` with no analysis logic — the same
contract `cgg-py` holds. Output is byte-identical to the CLI's, and a test
asserts that across all four formats.

## Why strings, not structs

Options arrive as a JSON document and results leave as strings. That is
what lets **one** ABI serve C, .NET, Java, Go, Ruby and anything else with
an FFI: adding a cgg flag adds a JSON key, not an entry point, so this
header does not change when cgg gains a feature and no wrapper needs
rebuilding.

It is affordable because rendering is nearly free next to analysis.
Measured on cgg's own tree: analysis 137.7 ms, `to_json()` 6.5 ms (4.7%),
`to_mermaid()` 1.3 ms (0.9%).

`cgg_analyze` returns an opaque handle rather than a rendered string, so
asking for mermaid *and* json *and* the metrics costs one analysis, not
three.

## The surface

| Function | Purpose |
| --- | --- |
| `cgg_version()` | Library version. Static — do not free. |
| `cgg_analyze(options_json, &err)` | Analyze. Returns a handle, or NULL. |
| `cgg_graph_render(g, format, &err)` | `"mermaid"`, `"json"`, `"dot"`, `"graphml"`. |
| `cgg_graph_meta(g, &err)` | Counts, metrics and notices as one JSON object. |
| `cgg_graph_callable_count(g)` | The commonest probe, without parsing JSON. |
| `cgg_graph_free(g)` / `cgg_string_free(s)` | Release. NULL is a no-op. |

Six functions. Everything else rides in the options JSON — see `cgg.h` for
the full key list.

## Rules

* Free every `char*` with `cgg_string_free` and every `cgg_graph*` with
  `cgg_graph_free`. They come from this library's allocator; `free(3)` is
  wrong.
* Failure is `NULL` plus an owned message in `*err`. Pass `NULL` for `err`
  if you do not want it.
* **Unknown option keys are an error, not ignored.** C callers hand-write
  JSON, and a silently-dropped `"hopz"` is the failure mode this boundary
  is most exposed to.
* Panics are caught at every entry point. A Rust panic unwinding into C is
  undefined behaviour, so it becomes an error instead.
* `cgg_analyze` is safe to call concurrently, and a handle is immutable and
  renderable from several threads at once.

## Building

```bash
cargo build --release -p cgg-ffi
# target/release/libcgg.so   (cdylib — dlopen or link)
# target/release/libcgg.a    (staticlib — stay a single binary)
```

The header is checked in at `include/cgg.h`; it is hand-written rather than
generated, because it is documentation as much as declarations.

**Static linking keeps the promise the CLI makes.** A C program linked
against `libcgg.a` depends on nothing but libc and libgcc — verified with
`ldd`. That is the point: cgg is offline and self-contained, and a binding
should not be the thing that changes it.

## Size

The library is ~99 MB on disk, of which ~95 MB is `.rodata` — the parse
tables for 44 tree-sitter grammars. `.text` is under 7 MB. It is already
stripped, so that floor is the grammars, not the code. It compresses about
10:1 (9.4 MB gzip, 5.6 MB xz), so the download a package registry serves is
roughly 10 MB.

## Known issue

`cgg::analyze` leaks about **161 bytes per call** — a fixed, one-time cost
at parser setup inside tree-sitter's C allocations, not per file: 0 bytes
for a run that parses nothing, 161 for one file, 163 for several,
independent of thread count. It does not accumulate with tree size, but it
does accumulate across calls, so a long-lived host process analyzing on a
loop will grow slowly (~14 MB after a million analyses). It is not specific
to this crate — a pure Rust consumer of `cgg::analyze` leaks identically —
and it was unreachable before 0.6.0, when one analysis per process was the
only option. Tracked for a fix.
