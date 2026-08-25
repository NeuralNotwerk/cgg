// Tests for the Node bindings.
//
// Run with `node --test tests/` after `npm run build`. Uses node:test so
// there is no test-framework dependency to keep current.
//
// The bar these hold is the one `crates/cgg-py/tests` holds: the module
// must agree with the binary byte for byte. A binding that analyses
// *nearly* the same graph as the CLI is worse than no binding, because
// the disagreement is invisible until someone acts on it.

const test = require("node:test");
const assert = require("node:assert");
const { execFileSync } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs");

const cgg = require("../index.js");

const REPO = path.resolve(__dirname, "../../..");
const TREE = path.join(REPO, "crates/cgg-walk");
// `cgg-walk` is one file, so every level folds it to a single node with
// no edges left. Rollup assertions need a tree with cross-file calls.
const MULTI = path.join(REPO, "crates/cgg");
// Set by CI, and by scripts/publish-node.sh, to the binary built from
// this same commit. Parity against a *different* build proves nothing.
const BIN = process.env.CGG_BIN || path.join(REPO, "target/release/cgg");

const haveBin = fs.existsSync(BIN);
// `-t json` and the audit sidecar embed per-file parse timings, so they
// differ between any two runs. Everything else is byte-identical.
const stripMs = (s) => s.replace(/"[a-z_]*ms":\s*[0-9.]+/g, '"ms":X');
const cli = (args) =>
  execFileSync(BIN, args, { maxBuffer: 1 << 28 }).toString();

test("version matches the crate", () => {
  assert.match(cgg.version(), /^\d+\.\d+\.\d+$/);
});

test("languages() lists every plugin", () => {
  const langs = cgg.languages();
  assert.ok(langs.length >= 44, `expected >= 44 languages, got ${langs.length}`);
  assert.ok(langs.includes("rust") && langs.includes("python"));
  assert.strictEqual(new Set(langs).size, langs.length, "duplicate language id");
});

test("analyze() resolves to a graph", async () => {
  const g = await cgg.analyze(TREE);
  assert.ok(g.callableCount > 0);
  assert.strictEqual(g.callables.length, g.callableCount);
  assert.ok(g.edges.length > 0);
  assert.ok(g.files.length > 0);
});

test("analyzeSync() agrees with analyze()", async () => {
  const a = await cgg.analyze(TREE);
  const b = cgg.analyzeSync(TREE);
  assert.strictEqual(a.toMermaid(), b.toMermaid());
});

test("accepts one path or several", async () => {
  const one = await cgg.analyze(TREE);
  const many = await cgg.analyze([TREE, path.join(REPO, "crates/cgg-format")]);
  assert.ok(many.callableCount > one.callableCount);
});

test("callables and edges carry the documented shape", async () => {
  const g = await cgg.analyze(TREE);
  const c = g.callables[0];
  for (const k of ["id", "qualifiedName", "simpleName", "kind", "language",
                   "file", "startLine", "endLine", "visibility", "synthetic"]) {
    assert.ok(k in c, `callable missing ${k}`);
  }
  // The field names mirror the Python module deliberately.
  const e = g.edges[0];
  for (const k of ["src", "dst", "siteLine", "siteByte", "confidence", "via"]) {
    assert.ok(k in e, `edge missing ${k}`);
  }
  assert.ok(["high", "medium", "low"].includes(e.confidence));
  // Ids are content-derived hashes now, not positional indices, so
  // membership in the file list is what's checkable — not a range.
  assert.ok(
    g.files.some((f) => f.id === c.file),
    "callable's file id does not match any analyzed file",
  );
  // Not a debug-formatted string: 0.6.2 briefly shipped `"\"pub\""`.
  assert.ok(!c.visibility.includes('"'), `visibility is quoted: ${c.visibility}`);
});

test("a rolled-up graph carries its group metadata", async () => {
  // REGRESSION: the option keywords reached the pipeline but `rollup`
  // was never added to the JS `Callable`, so Node could ask for a
  // rollup and not see that it got one. The `weight` on a folded edge —
  // the call count the fold exists to preserve — was dropped silently.
  const g = await cgg.analyze(MULTI, { rollupBy: "file" });
  const groups = g.callables.filter((c) => c.rollup);
  assert.ok(groups.length > 0, "no group nodes came back");
  assert.ok(
    groups.every((c) => c.kind === "group"),
    "a group node must not claim to be a function or a method",
  );
  const r = groups[0].rollup;
  for (const k of ["level", "members", "files", "languages",
                   "internalCalls", "unreferencedMembers"]) {
    assert.ok(k in r, `rollup missing ${k}`);
  }
  assert.strictEqual(r.level, "file");
  assert.ok(r.members >= 1);
  assert.ok(
    Math.max(...g.edges.map((e) => e.weight)) > 1,
    "a folded edge must report the call count it stands for",
  );
});

test("an ordinary graph carries no rollup metadata at all", async () => {
  const g = await cgg.analyze(MULTI);
  assert.ok(g.callables.every((c) => !c.rollup), "rollup leaked into a plain graph");
  assert.ok(g.edges.every((e) => e.weight === 1), "weight leaked into a plain graph");
});

test("a token budget folds the graph and a met one does not", async () => {
  const tight = await cgg.analyze(path.join(REPO, "crates"), { rollup: "5k" });
  assert.ok(tight.callables.some((c) => c.rollup), "5k must force a fold");
  const loose = await cgg.analyze(MULTI, { rollup: "10m" });
  assert.ok(loose.callables.every((c) => !c.rollup), "a met budget must fold nothing");
});

test("fromGraph replays a saved graph identically", async () => {
  const plain = await cgg.analyze(MULTI);
  const file = path.join(fs.mkdtempSync(path.join(require("node:os").tmpdir(), "cgg-")), "g.json");
  fs.writeFileSync(file, plain.toJson());
  const replay = await cgg.analyze([], { fromGraph: file, rollupBy: "file" });
  const direct = await cgg.analyze(MULTI, { rollupBy: "file" });
  const key = (g) => g.callables.map((c) => `${c.qualifiedName}`).join("|");
  assert.strictEqual(key(replay), key(direct), "replay does not match a direct run");
  assert.deepStrictEqual(
    replay.edges.map((e) => e.weight),
    direct.edges.map((e) => e.weight),
  );
});

test("a bad rollup value is rejected by name", async () => {
  for (const [kw, bad] of [["rollupBy", "modul"], ["rollup", "banana"],
                           ["rollupFormat", "yaml"]]) {
    await assert.rejects(
      () => cgg.analyze(MULTI, { [kw]: bad }),
      (e) => typeof e.message === "string" && e.message.length > 0,
      `${kw}="${bad}" was accepted`,
    );
  }
});

test("metrics and notices survive the boundary", async () => {
  const g = await cgg.analyze(TREE);
  assert.ok(g.metrics.filesAnalyzed >= 1);
  assert.ok(g.metrics.wallMs > 0);
  assert.ok(g.notices.some((n) => n.includes("callables")),
    "the run summary should be among the notices");
});

test("jobs is honoured, and observable", async () => {
  for (const jobs of [1, 3]) {
    const g = await cgg.analyze(TREE, { jobs });
    assert.strictEqual(g.jobs, jobs);
  }
});

test("options actually change the graph", async () => {
  const base = await cgg.analyze(TREE);
  const wider = await cgg.analyze(TREE, { includeExternal: true, includeStdlib: true });
  assert.ok(wider.callableCount > base.callableCount, "exit nodes were not added");
  const narrowed = await cgg.analyze(TREE, { filter: ["walk$"], hops: 1 });
  assert.ok(narrowed.callableCount < base.callableCount, "filter did not narrow");
});

test("a bad option is rejected by name", async () => {
  await assert.rejects(
    () => cgg.analyze(TREE, { deadCodeConfidence: "nope" }),
    (e) => e.message.includes("deadCodeConfidence"),
  );
});

test("a missing path is an Error, not a crash", async () => {
  await assert.rejects(
    () => cgg.analyze("/definitely/not/here"),
    (e) => e.message.includes("does not exist"),
  );
  // The process must still be usable afterwards.
  assert.ok((await cgg.analyze(TREE)).callableCount > 0);
});

test("concurrent analyses do not interfere", async () => {
  const [a, b, c] = await Promise.all([
    cgg.analyze(TREE, { jobs: 2 }),
    cgg.analyze(TREE, { includeStdlib: true }),
    cgg.analyze(TREE, { jobs: 2 }),
  ]);
  assert.strictEqual(a.toMermaid(), c.toMermaid());
  assert.notStrictEqual(a.toMermaid(), b.toMermaid());
});

test("repeated calls are identical", async () => {
  const a = await cgg.analyze(TREE);
  const b = await cgg.analyze(TREE);
  assert.strictEqual(a.toMermaid(), b.toMermaid());
});

// --- parity with the binary -------------------------------------------
// The whole reason a binding is trustworthy. Skipped, loudly, when no
// binary is available rather than silently passing.

test("renderers are byte-identical to the CLI", { skip: !haveBin && `no binary at ${BIN}` }, async () => {
  const g = await cgg.analyze(TREE);
  for (const [fmt, out] of [
    ["mermaid", g.toMermaid()],
    ["json", g.toJson()],
    ["dot", g.toDot()],
    ["graphml", g.toGraphml()],
  ]) {
    assert.strictEqual(stripMs(out), stripMs(cli([TREE, "-t", fmt])), `${fmt} differs from the CLI`);
  }
});

test("options match the equivalent CLI flags", { skip: !haveBin && `no binary at ${BIN}` }, async () => {
  const cases = [
    [{ includeExternal: true, includeStdlib: true }, ["--include-external", "--include-stdlib"]],
    [{ dynamicDispatch: true }, ["--dynamic-dispatch"]],
    [{ referenceEdges: true }, ["--reference-edges"]],
    [{ entryNodes: false }, ["--no-entry-nodes"]],
    [{ lang: ["rust"] }, ["--lang", "rust"]],
    [{ includeTests: true }, ["--include-tests"]],
    [{ filter: ["walk$"], hops: 1 }, ["--filter", "walk$", "-n", "1"]],
  ];
  let changedAny = false;
  const baseline = stripMs(cli([TREE, "-t", "json"]));
  for (const [opts, flags] of cases) {
    const mine = stripMs((await cgg.analyze(TREE, opts)).toJson());
    const theirs = stripMs(cli([TREE, "-t", "json", ...flags]));
    assert.strictEqual(mine, theirs, `${JSON.stringify(opts)} != ${flags.join(" ")}`);
    if (theirs !== baseline) changedAny = true;
  }
  // Without this the test would pass while the module ignored every
  // keyword it was given.
  assert.ok(changedAny, "no option changed the graph — parity was vacuous");
});

test("mermaid numbers its nodes by default", async () => {
  const g = await cgg.analyze(TREE);
  const text = g.toMermaid();
  assert.match(text, /^ {2}N\d+\[/m, "want numbered node ids");
  assert.doesNotMatch(text, /^ {2}C[0-9a-z]{4,}\[/m, "want no hashed ids");
});

test("nodeIds 'hash' gives back the ids toJson carries", async () => {
  const g = await cgg.analyze(TREE);
  const text = g.toMermaid("hash");
  const ids = Object.keys(JSON.parse(g.toJson()).callables);
  assert.ok(ids.length > 0, "fixture produced no callables");
  for (const id of ids) {
    assert.ok(text.includes(`  ${id}[`), `${id} missing from hashed mermaid`);
  }
});

test("nodeIds 'short' is the explicit spelling of the default", async () => {
  const g = await cgg.analyze(TREE);
  assert.strictEqual(g.toMermaid("short"), g.toMermaid());
});

test("a bad nodeIds value is rejected by name", async () => {
  const g = await cgg.analyze(TREE);
  assert.throws(() => g.toMermaid("sequential"), /sequential|short/);
});

test("the node-id scheme changes only the names", async () => {
  const g = await cgg.analyze(MULTI);
  const short = g.toMermaid();
  const hashed = g.toMermaid("hash");
  const labels = (s) => [...s.matchAll(/\["([^"]*)"\]/g)].map((m) => m[1]);
  assert.deepStrictEqual(labels(short), labels(hashed));
  assert.strictEqual(short.split("\n").length, hashed.split("\n").length);
});

test("toMermaid matches the CLI, both schemes", { skip: !haveBin }, async () => {
  const g = await cgg.analyze(TREE);
  assert.strictEqual(g.toMermaid(), cli([TREE, "-t", "mermaid"]));
  assert.strictEqual(
    g.toMermaid("hash"),
    cli([TREE, "-t", "mermaid", "--node-ids", "hash"]),
  );
});
