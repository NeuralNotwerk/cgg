# cgg-resolve — Call-site resolution for cgg

An internal crate of [**cgg**](https://github.com/NeuralNotwerk/cgg), an
offline, deterministic call-graph generator for 44 languages.

Links call sites to definitions: type propagation, intra-file scoping,
cross-file import chains, FFI edges, descriptor `$ref` edges, framework entry
points, and optional dynamic-dispatch fan-out. Every edge carries a confidence
level and the id of the resolver that produced it.

## You probably want `cgg` instead

This crate is published so that `cgg` can depend on it by version. Its API is
**pre-1.0 and changes freely between minor releases** to serve that one
consumer — it is not designed as a standalone library.

To generate call graphs, use the [`cgg`](https://crates.io/crates/cgg) crate
or the CLI:

```bash
cargo install cgg
cgg ./src -o graph.mmd
```

Licensed Apache-2.0 OR MIT.
