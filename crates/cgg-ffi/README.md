# cgg-ffi — the C ABI

One shared library, every language that can call C.

```c
#include <stdio.h>   /* cgg.h pulls in <stdint.h> only */
#include "cgg.h"

int main(void) {
    char *err = NULL;
    cgg_graph *g = cgg_analyze("{\"paths\":[\"./src\"]}", &err);
    if (!g) { fputs(err, stderr); cgg_string_free(err); return 1; }

    char *mermaid = cgg_graph_render(g, "mermaid", &err);
    puts(mermaid);

    cgg_string_free(mermaid);
    cgg_graph_free(g);
    return 0;
}
```

```bash
cargo build --release -p cgg-ffi
cc -I crates/cgg-ffi/include main.c -o demo -L target/release -lcgg
```

A translation layer over `cgg::analyze` with no analysis logic — the same
contract `cgg-py` holds. `mermaid`, `dot` and `graphml` come out
byte-identical to the CLI's; `json` matches too apart from the per-run
timings it embeds (`parse_ms`, `wall_ms`), which no two runs of any build
share. The in-crate tests check that every format renders and round-trips
through the ABI; the byte-for-byte comparison against the binary lives in
the `cgg-py` and `cgg-node` test suites, over the same `cgg::analyze` this
crate wraps.

## Why strings, not structs

Options arrive as a JSON document and results leave as strings. That is
what lets **one** ABI serve C, .NET, Java, Go, Ruby and anything else with
an FFI: adding a cgg flag adds a JSON key, not an entry point, so this
header does not change when cgg gains a feature and no wrapper needs
rebuilding.

It is affordable because rendering is nearly free next to analysis.
Measured through this ABI on cgg's own `crates/` (2,019 callables, median
of 11): analysis 127 ms, `cgg_graph_render(g, "json")` 6.7 ms (5.3%) for
2.3 MB, `"mermaid"` 1.3 ms (1.0%) for 184 KB.

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

Seven exported symbols in all (the last row is two). Everything else rides
in the options JSON — see `cgg.h` for the full key list.

## Rules

* Free every `char*` with `cgg_string_free` and every `cgg_graph*` with
  `cgg_graph_free`. They come from this library's allocator; `free(3)` is
  wrong.
* Failure is `NULL` plus an owned message in `*err`. Pass `NULL` for `err`
  if you do not want it.
* **Unknown option keys are an error, not ignored.** C callers hand-write
  JSON, and a silently-dropped `"hopz"` is the failure mode this boundary
  is most exposed to.
* Panics are caught at every entry point that runs any logic — all six,
  via one `catch_unwind` guard. (`cgg_version` returns a pointer to a
  literal and has nothing to catch.) A Rust panic unwinding into C is
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

`libcgg.so` is 99 MiB on disk, of which 91 MiB is `.rodata` — the parse
tables for 44 tree-sitter grammars. `.text` is 6.4 MiB. It is already
stripped, so that floor is the grammars, not the code. It compresses about
11:1 (9.3 MiB gzip -9, 5.5 MiB xz -9), so the download a package registry
serves is roughly 10 MB. `libcgg.a` is larger — 125 MiB — because a static
archive carries every object whether or not it is linked in.

## Long-lived hosts

`cgg_analyze` is safe to call in a loop. 0.6.0 leaked ~161 bytes per call
through a `Box::leak` in the type-hint resolver whose justification —
"we're in a short-lived analysis pass" — stopped being true the moment the
pipeline became embeddable. **Fixed in 0.6.1**: valgrind reports 0 bytes
definitely lost on `cgg-walk`, `cgg-format`, a single 467-line file and
all of `./crates`, where 0.6.0 lost 161 bytes in 28 blocks. Forty
back-to-back `cgg_analyze` / `cgg_graph_free` cycles on this build plateau
at ~15 MB RSS rather than climbing.

If you profile this library for leaks yourself, note that **`mimalloc`
hides them from valgrind**: build with the `#[global_allocator]` removed,
or valgrind will report nothing at all. That is exactly how the original
leak escaped notice.
