//! Directory walker.
//!
//! Produces a stream of [`FileCandidate`]s plus [`Skip`] records for
//! every path that was discovered but not analyzed. Both are rolled
//! up into the audit log.
//!
//! Behavior layers (each layer may reject a path):
//!
//! 1. Built-in deny directories: `node_modules`, `.venv`, `venv`,
//!    `site-packages`, `vendor`, `target`, `build`, `bin`, `obj`,
//!    `dist`, `.git`, `.gradle`, `.cargo`, `__pycache__`, `.next`,
//!    `.nuxt`. Matched by exact directory name anywhere in the path.
//! 2. `.gitignore` walked up the tree (via [`ignore`] defaults).
//! 3. `.cggignore` parsed at every directory boundary
//!    (gitignore-syntax).
//! 4. Symlink-out-of-root detection.
//! 5. Binary-content heuristic (first 8KB: NUL byte present).
//! 6. Minified-source heuristic, **only when
//!    [`WalkConfig::skip_minified`] is set**, and then for
//!    `.js`/`.mjs`/`.cjs`/`.css` only: a `name.min.<ext>` filename, or
//!    an average line length over 2,000 bytes on the same 8KB probe
//!    used for the binary check. Bundled/minified files carry no
//!    useful callable structure and dominate wall time on a
//!    mixed-language tree — but they are still real, parseable source,
//!    so dropping them is the caller's decision and never the
//!    walker's. Off by default; every other layer above is
//!    unconditional.
//!
//! Unrecognized extensions are *not* filtered here — the walker emits
//! them with `language=None` and later stages (language detector)
//! classify them as `skip_reason: unknown-extension` in the audit.
//!
//! Every skip is reported so nothing is silently dropped.

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cgg_core::audit::SkipReason;

/// Directory segments that are always excluded regardless of user
/// configuration. Bypassable only via code changes.
pub const BUILTIN_DENY_DIRS: &[&str] = &[
    "node_modules",
    ".venv",
    "venv",
    "site-packages",
    "vendor",
    "target",
    "build",
    "bin",
    "obj",
    "dist",
    ".git",
    ".gradle",
    ".cargo",
    "__pycache__",
    ".next",
    ".nuxt",
];

/// Bytes read from each file head for the binary-content and
/// minified-source heuristics.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Extensions eligible for the minified-source heuristic (VERIFIED
/// §1k / §3.11). Deliberately narrow: these are the languages that
/// have a `.min.<ext>` build-artifact convention and where an
/// extreme average line length is a hard signal of bundled output
/// rather than of dense hand-written source.
const MINIFIABLE_EXTS: &[&str] = &["js", "mjs", "cjs", "css"];

/// Average bytes-per-line above which a minifiable-extension file is
/// treated as minified, even without a `.min.<ext>` filename.
const MINIFIED_AVG_LINE_LEN: usize = 2000;

/// A file the walker has decided to pass on to language detection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileCandidate {
    pub path: PathBuf,
    pub size_bytes: u64,
}

/// A file discovered but excluded from analysis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skip {
    pub path: PathBuf,
    pub reason: SkipReason,
}

/// Walker result combining produced candidates and skipped entries.
#[derive(Clone, Debug, Default)]
pub struct WalkOutcome {
    pub candidates: Vec<FileCandidate>,
    pub skips: Vec<Skip>,
}

impl WalkOutcome {
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty() && self.skips.is_empty()
    }
}

/// Configuration for [`walk`].
#[derive(Clone, Debug)]
pub struct WalkConfig {
    /// Roots to scan. Each must exist.
    pub roots: Vec<PathBuf>,
    /// Additional ignore file applied on top of built-ins + gitignore
    /// + .cggignore.
    pub extra_ignore_file: Option<PathBuf>,
    /// Follow symlinks. We still reject symlinks whose canonicalized
    /// target lies outside any root.
    pub follow_symlinks: bool,
    /// Byte threshold; files larger than this are skipped with
    /// `SkipReason::TooLarge`. `None` disables the check.
    pub max_file_size: Option<u64>,
    /// Apply the minified-source heuristic (`SkipReason::Minified`).
    ///
    /// **Default `false`**, unlike every other check here. The others
    /// reject files cgg cannot usefully analyze at all — too large to
    /// hold, binary, ignored by the user's own rules. A minified
    /// bundle is none of those: it is real source that parses, and
    /// skipping it removes callables the caller never asked to lose.
    /// So this one is opt-in, matching the rest of cgg's
    /// graph-content switches.
    pub skip_minified: bool,
}

impl Default for WalkConfig {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            extra_ignore_file: None,
            follow_symlinks: false,
            // 25 MiB — anything bigger is almost certainly generated.
            max_file_size: Some(25 * 1024 * 1024),
            // Opt-in: the default walk analyzes every file it can parse.
            skip_minified: false,
        }
    }
}

/// Walk every root and return candidates + skips.
pub fn walk(cfg: &WalkConfig) -> Result<WalkOutcome> {
    let mut out = WalkOutcome::default();
    let canonical_roots: Vec<PathBuf> = cfg
        .roots
        .iter()
        .map(|p| {
            fs::canonicalize(p)
                .with_context(|| format!("canonicalizing input path {}", p.display()))
        })
        .collect::<Result<_>>()?;

    for (root, canon) in cfg.roots.iter().zip(canonical_roots.iter()) {
        walk_one(root, canon, cfg, &mut out)?;
    }

    Ok(out)
}

fn walk_one(
    display_root: &Path,
    canonical_root: &Path,
    cfg: &WalkConfig,
    out: &mut WalkOutcome,
) -> Result<()> {
    // If the user passed a file directly, short-circuit: still apply
    // skip checks so single-file audits stay consistent.
    if display_root.is_file() {
        if let Some(reason) = builtin_reason(display_root) {
            out.skips.push(Skip {
                path: display_root.to_path_buf(),
                reason,
            });
            return Ok(());
        }
        if let Some(skip) = classify_file(display_root, cfg)? {
            out.skips.push(skip);
        } else {
            push_candidate(display_root, out)?;
        }
        return Ok(());
    }

    let mut builder = WalkBuilder::new(display_root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .parents(true)
        .follow_links(cfg.follow_symlinks)
        .add_custom_ignore_filename(".cggignore");

    if let Some(extra) = &cfg.extra_ignore_file {
        builder.add_ignore(extra);
    }

    // Sort, or the graph depends on readdir order.
    //
    // Without this the walker yields whatever order the filesystem hands
    // back, which differs between machines for byte-identical trees.
    // Node ids are positional, so the same commit produced a different
    // `C0`/`C1`/… assignment and a different declaration order on each
    // one — caught by running the same fixture through five distribution
    // channels in five containers and getting three distinct graphs.
    //
    // `--jobs` determinism never saw this: it varies thread count against
    // one directory on one host, where readdir order is a constant.
    // "Deterministic" and "diffable in a PR" both mean across machines,
    // not just across runs.
    builder.sort_by_file_path(std::path::Path::cmp);

    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                // ignore-crate errors don't expose a uniform path
                // accessor; extract one if the variant carries one,
                // otherwise fall back to the root path.
                let path =
                    extract_err_path(&err).unwrap_or_else(|| display_root.to_path_buf());
                out.skips.push(Skip {
                    path,
                    reason: SkipReason::ParseError(err.to_string()),
                });
                continue;
            }
        };

        let path = entry.path();

        // Directories that aren't root are handled implicitly by the
        // walker descending into them. We only act on files.
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        // Built-in deny check — belt-and-suspenders in case a path
        // bypassed ignore filtering (e.g. symlinks).
        if let Some(reason) = builtin_reason(path) {
            out.skips.push(Skip {
                path: path.to_path_buf(),
                reason,
            });
            continue;
        }

        // Symlink-out-of-root check.
        if entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false)
            || is_symlink_chain(path)
        {
            match fs::canonicalize(path) {
                Ok(target) => {
                    if !target.starts_with(canonical_root) {
                        out.skips.push(Skip {
                            path: path.to_path_buf(),
                            reason: SkipReason::SymlinkOutsideRoot,
                        });
                        continue;
                    }
                }
                Err(err) => {
                    out.skips.push(Skip {
                        path: path.to_path_buf(),
                        reason: SkipReason::ParseError(err.to_string()),
                    });
                    continue;
                }
            }
        }

        if let Some(skip) = classify_file(path, cfg)? {
            out.skips.push(skip);
        } else {
            push_candidate(path, out)?;
        }
    }

    Ok(())
}

fn push_candidate(path: &Path, out: &mut WalkOutcome) -> Result<()> {
    let md = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    out.candidates.push(FileCandidate {
        path: path.to_path_buf(),
        size_bytes: md.len(),
    });
    Ok(())
}

fn is_symlink_chain(p: &Path) -> bool {
    fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Return a skip reason if the file fails a per-file check
/// (size, minified-filename, binary sniffing, minified-line-length).
/// Returns `None` if the file is acceptable.
///
/// Both minified checks are gated on [`WalkConfig::skip_minified`];
/// the rest are unconditional.
fn classify_file(path: &Path, cfg: &WalkConfig) -> Result<Option<Skip>> {
    let md = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if let Some(max) = cfg.max_file_size
        && md.len() > max
    {
        return Ok(Some(Skip {
            path: path.to_path_buf(),
            reason: SkipReason::TooLarge,
        }));
    }

    // `None` when the caller did not ask for the heuristic, so both
    // branches below fall through and nothing pays for the check.
    let minifiable_ext = if cfg.skip_minified {
        minifiable_extension(path)
    } else {
        None
    };

    // The filename convention is decided from the path alone, before
    // any read — cheapest check first.
    if let Some(ext) = minifiable_ext
        && has_min_dot_extension(path, ext)
    {
        return Ok(Some(Skip {
            path: path.to_path_buf(),
            reason: SkipReason::Minified,
        }));
    }

    // One probe read serves both the binary heuristic and the
    // average-line-length heuristic below.
    let probe = read_probe(path)?;
    if probe.contains(&0) {
        return Ok(Some(Skip {
            path: path.to_path_buf(),
            reason: SkipReason::Binary,
        }));
    }

    if minifiable_ext.is_some() && average_line_len(&probe) > MINIFIED_AVG_LINE_LEN {
        return Ok(Some(Skip {
            path: path.to_path_buf(),
            reason: SkipReason::Minified,
        }));
    }

    Ok(None)
}

/// Read up to [`BINARY_SNIFF_BYTES`] from the head of `path`.
fn read_probe(path: &Path) -> Result<Vec<u8>> {
    let mut buf = [0u8; BINARY_SNIFF_BYTES];
    let mut f =
        fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let n = f
        .read(&mut buf)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(buf[..n].to_vec())
}

/// The file's extension, if it is one of [`MINIFIABLE_EXTS`].
fn minifiable_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    MINIFIABLE_EXTS.iter().find(|&&e| e == ext).copied()
}

/// `name.min.<ext>` — the filename convention bundlers use to mark an
/// already-minified build artifact.
fn has_min_dot_extension(path: &Path, ext: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(&format!(".min.{ext}")))
}

/// Average bytes-per-line over `buf` (bytes/lines). A buffer with no
/// newline counts as one line, so a single unbroken minified line is
/// measured at its own full length rather than dividing by zero.
fn average_line_len(buf: &[u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let lines = buf.iter().filter(|&&b| b == b'\n').count().max(1);
    buf.len() / lines
}

/// Match any component of `path` against the built-in deny list.
fn builtin_reason(path: &Path) -> Option<SkipReason> {
    for comp in path.components() {
        if let Some(name) = comp.as_os_str().to_str()
            && BUILTIN_DENY_DIRS.contains(&name)
        {
            return Some(SkipReason::Builtin(name.to_string()));
        }
    }
    None
}

/// Walk an [`ignore::Error`] tree looking for a `WithPath { path, .. }`
/// layer; return the first path found, if any.
fn extract_err_path(err: &ignore::Error) -> Option<PathBuf> {
    use ignore::Error as E;
    match err {
        E::WithPath { path, .. } => Some(path.clone()),
        E::WithLineNumber { err, .. } => extract_err_path(err),
        E::WithDepth { err, .. } => extract_err_path(err),
        E::Partial(list) => list.iter().find_map(extract_err_path),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, body: &[u8]) {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::File::create(&p).unwrap().write_all(body).unwrap();
    }

    #[test]
    fn discovers_plain_files() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "a.py", b"print('hi')\n");
        write(tmp.path(), "b.rs", b"fn x() {}\n");

        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        assert_eq!(out.candidates.len(), 2);
        assert!(out.skips.is_empty());
    }

    #[test]
    fn builtin_deny_skips_node_modules_and_target() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "src/a.rs", b"fn x() {}\n");
        write(
            tmp.path(),
            "node_modules/lodash.js",
            b"module.exports={};\n",
        );
        write(tmp.path(), "target/debug/out.bin", b"binary-looking\n");

        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        assert!(out.candidates.iter().any(|c| c.path.ends_with("a.rs")));
        assert!(
            out.skips.iter().any(
                |s| matches!(&s.reason, SkipReason::Builtin(d) if d == "node_modules")
            )
        );
        assert!(
            out.skips
                .iter()
                .any(|s| matches!(&s.reason, SkipReason::Builtin(d) if d == "target"))
        );
    }

    #[test]
    fn cggignore_is_honored() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "keep.py", b"pass\n");
        write(tmp.path(), "drop.py", b"pass\n");
        write(tmp.path(), ".cggignore", b"drop.py\n");

        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        let kept: Vec<_> = out
            .candidates
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(kept.contains(&"keep.py".to_string()));
        assert!(!kept.contains(&"drop.py".to_string()));
        // Note: `.cggignore` itself is currently emitted as a
        // candidate because it isn't a source extension — the
        // language detector (Task 3) handles unknown-extension skips.
    }

    #[test]
    fn gitignore_is_honored() {
        let tmp = TempDir::new().unwrap();
        // Initialize a minimal git dir so .gitignore activates.
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        write(tmp.path(), ".gitignore", b"secret.py\n");
        write(tmp.path(), "secret.py", b"pw='1'\n");
        write(tmp.path(), "visible.py", b"pass\n");

        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        let kept: Vec<_> = out
            .candidates
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(kept.contains(&"visible.py".to_string()));
        assert!(!kept.contains(&"secret.py".to_string()));
    }

    #[test]
    fn binary_file_is_skipped() {
        let tmp = TempDir::new().unwrap();
        // NUL byte inside -> binary heuristic trips.
        write(tmp.path(), "blob.dat", b"head\x00tail");
        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        assert!(
            out.skips
                .iter()
                .any(|s| matches!(s.reason, SkipReason::Binary))
        );
    }

    #[test]
    fn too_large_is_skipped() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "big.py", &vec![b'a'; 32 * 1024]);
        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            // 16 KiB cap for the test
            max_file_size: Some(16 * 1024),
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        assert!(
            out.skips
                .iter()
                .any(|s| matches!(s.reason, SkipReason::TooLarge))
        );
    }

    /// Write the three-file minified fixture: a `.min.js` by name, a
    /// `.js` whose single 3,000-byte line trips the average-line-length
    /// rule, and an ordinary multi-line `.js` that must survive both.
    fn minified_fixture() -> TempDir {
        let tmp = TempDir::new().unwrap();
        // Filename convention: `name.min.<ext>`.
        write(tmp.path(), "a.min.js", b"function f(){return 1}\n");
        // No `.min.` in the name, but a single 3,000-byte line — over
        // the 2,000-byte average-line-length threshold on the probe.
        let long_line: Vec<u8> = vec![b'x'; 3000];
        write(tmp.path(), "b.js", &long_line);
        // Ordinary multi-line source: must NOT be skipped either way.
        write(
            tmp.path(),
            "c.js",
            b"function add(a, b) {\n  return a + b;\n}\n",
        );
        tmp
    }

    /// The default walk analyzes minified files like any other source.
    ///
    /// The heuristic is accurate, and that is exactly why this test
    /// exists: an accurate filter that nobody switched on still removes
    /// callables the caller never agreed to lose. Every other cgg
    /// switch that changes what lands in the graph is opt-in, and this
    /// is the only one that *removes*.
    #[test]
    fn minified_files_are_analyzed_by_default() {
        let tmp = minified_fixture();
        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();

        assert!(
            !out.skips
                .iter()
                .any(|s| matches!(s.reason, SkipReason::Minified)),
            "no file may be skipped as Minified without --skip-minified; \
             skips: {:?}",
            out.skips
        );
        let analyzed: Vec<String> = out
            .candidates
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        for want in ["a.min.js", "b.js", "c.js"] {
            assert!(
                analyzed.contains(&want.to_string()),
                "{want} must be analyzed at the default; analyzed: {analyzed:?}"
            );
        }
    }

    /// VERIFIED §1k / §3.11 (change c9): with `skip_minified` set, a
    /// `.min.js` filename and an extreme-average-line-length `.js` file
    /// are both skipped as `Minified`, while an ordinary `.js` file is
    /// analyzed normally.
    #[test]
    fn minified_js_is_skipped_by_name_and_by_line_length() {
        let tmp = minified_fixture();

        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            skip_minified: true,
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();

        let skipped_minified: Vec<String> = out
            .skips
            .iter()
            .filter(|s| matches!(s.reason, SkipReason::Minified))
            .map(|s| s.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            skipped_minified.contains(&"a.min.js".to_string()),
            "a.min.js should be skipped as Minified by filename; skipped: {skipped_minified:?}"
        );
        assert!(
            skipped_minified.contains(&"b.js".to_string()),
            "b.js should be skipped as Minified by average line length; skipped: {skipped_minified:?}"
        );

        let analyzed: Vec<String> = out
            .candidates
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            analyzed.contains(&"c.js".to_string()),
            "c.js is ordinary source and must be analyzed, not skipped; analyzed: {analyzed:?}"
        );
    }

    #[test]
    fn single_file_input_works() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "solo.rs", b"fn main() {}\n");
        let cfg = WalkConfig {
            roots: vec![tmp.path().join("solo.rs")],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        assert_eq!(out.candidates.len(), 1);
    }

    /// The walk must not depend on filesystem enumeration order.
    ///
    /// Shipped through 0.6.4: `WalkBuilder::build()` yields readdir
    /// order, which differs between machines for byte-identical trees.
    /// Node ids downstream are positional, so the same commit produced
    /// different ids and a different declaration order on each machine.
    /// Found by running one fixture through five distribution channels
    /// in five containers and getting three distinct graphs.
    ///
    /// The `--jobs` determinism test could not catch it: it varies
    /// thread count against one directory on one host, where readdir
    /// order is a constant.
    ///
    /// Asserts the property directly rather than trying to provoke a
    /// shuffle — creation order is the only lever available, and on a
    /// name-hashing filesystem it is not one.
    #[test]
    fn walk_order_is_sorted_not_readdir() {
        let tmp = TempDir::new().unwrap();
        // Neither sorted nor reverse-sorted on creation.
        for name in ["m.rs", "a.rs", "z.rs", "b.rs", "c.rs"] {
            write(tmp.path(), name, b"fn f() {}\n");
        }
        let cfg = WalkConfig {
            roots: vec![tmp.path().to_path_buf()],
            ..Default::default()
        };
        let out = walk(&cfg).unwrap();
        let names: Vec<String> = out
            .candidates
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            names, sorted,
            "walk yielded readdir order, not sorted order — the graph \
             will differ between machines for the same tree"
        );
    }
}
