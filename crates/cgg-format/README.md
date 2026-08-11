# cgg-format — Output formatters for cgg

An internal crate of [**cgg**](https://github.com/NeuralNotwerk/cgg), an
offline, deterministic call-graph generator for 44 languages.

Renders a `Graph` as mermaid (the default), JSON, DOT or GraphML.

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
