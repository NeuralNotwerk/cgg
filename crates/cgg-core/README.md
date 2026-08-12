# cgg-core — Core types for cgg

An internal crate of [**cgg**](https://github.com/NeuralNotwerk/cgg), an
offline, deterministic call-graph generator for 44 languages.

The substrate: `Graph`, the `CallableId` / `FileId` / `ResolverId` newtypes,
the audit schema, facts, framework rules, and stdlib lookup tables. Every
other cgg crate depends on this one; it depends on none of them.

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
