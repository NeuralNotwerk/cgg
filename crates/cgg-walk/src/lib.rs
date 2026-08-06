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

/// Bytes read from each file head for the binary-content heuristic.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

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
}

impl Default for WalkConfig {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            extra_ignore_file: None,
            follow_symlinks: false,
            // 25 MiB — anything bigger is almost certainly generated.
            max_file_size: Some(25 * 1024 * 1024),
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
/// (size, binary sniffing). Returns `None` if the file is acceptable.
fn classify_file(path: &Path, cfg: &WalkConfig) -> Result<Option<Skip>> {
    let md = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if let Some(max) = cfg.max_file_size
        && md.len() > max {
            return Ok(Some(Skip {
                path: path.to_path_buf(),
                reason: SkipReason::TooLarge,
            }));
        }
    if is_binary(path)? {
        return Ok(Some(Skip {
            path: path.to_path_buf(),
            reason: SkipReason::Binary,
        }));
    }
    Ok(None)
}

/// Binary heuristic: a NUL byte within the first 8KB signals binary
/// data. Fast, language-agnostic, and matches `git`'s own heuristic.
fn is_binary(path: &Path) -> Result<bool> {
    let mut buf = [0u8; BINARY_SNIFF_BYTES];
    let mut f =
        fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let n = f
        .read(&mut buf)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(buf[..n].contains(&0))
}

/// Match any component of `path` against the built-in deny list.
fn builtin_reason(path: &Path) -> Option<SkipReason> {
    for comp in path.components() {
        if let Some(name) = comp.as_os_str().to_str()
            && BUILTIN_DENY_DIRS.contains(&name) {
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
}
