// Load the built N-API module and make it analyse a real tree.
//
// The release workflow runs this against every .node it builds, and
// scripts/release.sh runs it against the local build, so a check that would
// fail the tag fails here first (0.11.0's first tag run died on a mermaid
// assertion nothing local had ever executed).
//
// Usage: node scripts/smoke/node_smoke.js MODULE_DIR [TREE]
//   MODULE_DIR  the directory holding index.js and package.json
//   TREE        what to analyse (default: crates/cgg-walk next to MODULE_DIR)
'use strict';
const path = require('path');

const dir = path.resolve(process.argv[2] || 'crates/cgg-node');
const tree = path.resolve(process.argv[3] || path.join(dir, '..', 'cgg-walk'));
const cgg = require(path.join(dir, 'index.js'));
const pkg = require(path.join(dir, 'package.json'));

// The version is ASSERTED, not printed. It used to be logged only, so a
// loader/native/package disagreement scrolled past in green. This reads the
// version out of the compiled .node — the one thing here that cannot be
// satisfied by a stale or hand-edited file.
const v = cgg.version();
if (v !== pkg.version) {
  throw new Error(
    'version mismatch: native binding reports ' + v +
    ' but package.json ships ' + pkg.version +
    ' — the .node was built from a different tree than the manifest');
}

const g = cgg.analyzeSync(tree);
if (g.callableCount < 1) { throw new Error('no callables'); }
// Entry nodes put a `%%` banner above the flowchart line.
if (!/^flowchart/m.test(g.toMermaid())) { throw new Error('bad mermaid'); }
console.log('ok', v, g.callableCount, 'callables');
