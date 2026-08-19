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
