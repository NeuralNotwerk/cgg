# Task 1 — Workspace skeleton, CLI, license gate

## What shipped

- Cargo workspace with 6 crates:
  `cgg-core`, `cgg-lang`, `cgg-resolve`, `cgg-format`, `cgg-walk`,
  `cgg` (binary).
- Full CLI surface (`clap` derive) matching the plan: positional
  paths, `-o`, `-t`, `--filter`, `-n`, `--max-paths`,
  `--include-tests`, `--ignore-file`, `--jobs`, `--cache`,
  `--no-cache`, `--lang`, `--audit-format`, `--metrics`, `-v`/`-q`.
- `cargo-deny` v0.16.4 installed and configured with a
  permissive-only license allow-list.

## Artifacts

- `cargo-deny.txt` — full license/bans/sources check output.
- `cargo-test.txt` — full workspace test run.
- `cgg-help.txt` — `cgg --help` output.

## Results

- `cargo-deny check licenses bans sources`: **bans ok, licenses ok,
  sources ok**. (Advisories skipped due to an upstream `cargo-deny`
  CVSS-4.0 parser issue in the RustSec DB; reinstated in CI once
  upstream fixes it.)
- `cargo test --workspace`: **all tests passing**.
- `cgg --help` renders 79 lines covering the full documented flag
  surface.
