//! Package manifests a plugin reads to learn the name its file is
//! importable under — the same job `rust.rs` does with `Cargo.toml`.
//!
//! The resolver needs to tell an import of a workspace package from an
//! import of a dependency: `import "github.com/google/uuid"` and
//! `require('@tryghost/errors')` name code that is not in the tree, and a
//! call through them must never be name-matched onto a workspace callable
//! that happens to share the method name. Only the manifest says which
//! names are the workspace's own: a Go import path is in the workspace
//! when it lies under a `go.mod` `module` path found in the tree, an npm
//! specifier when it names a `package.json` found in the tree.
//!
//! Both lookups walk up from the file to the nearest manifest, exactly as
//! the toolchains do, and read nothing else.

use std::path::Path;

/// The `module` path of the `go.mod` nearest above `path`, plus every
/// module a `replace` directive in it maps onto a local directory
/// (`replace example.com/lib => ../lib`): such a module is built from
/// source next to this one, not downloaded. Empty when no `go.mod` is
/// found (a GOPATH-era tree).
pub(crate) fn go_module_names(path: &Path) -> Vec<String> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        let gomod = d.join("go.mod");
        if gomod.is_file() {
            return std::fs::read_to_string(&gomod)
                .map(|text| parse_go_mod(&text))
                .unwrap_or_default();
        }
        dir = d.parent();
    }
    Vec::new()
}

/// `module` and locally-replaced module paths from a `go.mod` body.
fn parse_go_mod(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_replace_block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if in_replace_block {
            if line == ")" {
                in_replace_block = false;
                continue;
            }
            if let Some(m) = local_replace(line) {
                out.push(m);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("module") {
            let m = rest.trim().trim_matches('"');
            if !m.is_empty() && rest.starts_with(char::is_whitespace) {
                out.push(m.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("replace") {
            let rest = rest.trim();
            if rest == "(" {
                in_replace_block = true;
            } else if let Some(m) = local_replace(rest) {
                out.push(m);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `old [version] => ./dir` → `old`, when the replacement is a local path.
fn local_replace(spec: &str) -> Option<String> {
    let (old, new) = spec.split_once("=>")?;
    let new = new.trim().trim_matches('"');
    let local = new.starts_with("./") || new.starts_with("../") || new.starts_with('/');
    let old = old.split_whitespace().next()?.trim_matches('"');
    (local && !old.is_empty()).then(|| old.to_string())
}

/// Names a JS/TS file's code is imported under, besides relative
/// specifiers:
///
/// * the `name` of the nearest `package.json` that declares one — a
///   nameless manifest (`{"type": "module"}` in a sub-directory) makes no
///   package, so the walk goes on to the enclosing one;
/// * the `name` of every `deno.json` / `deno.jsonc` / `jsr.json` above the
///   file, and each of their import-map keys that maps into the tree
///   (`"@/": "./src/"`) — a Deno workspace imports its members by name;
/// * every `compilerOptions.paths` key of the nearest `tsconfig.json` /
///   `jsconfig.json`, following a relative `extends` chain (`"@app/*"`).
///
/// Path-alias keys keep their `*` or trailing `/`; the resolver reads
/// those as prefixes.
pub(crate) fn js_module_names(path: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut npm_done = false;
    let mut ts_done = false;
    let mut dir = path.parent();
    while let Some(d) = dir {
        if !npm_done
            && let Some(m) = read_jsonc(&d.join("package.json"))
            && let Some(name) = json_name(&m)
        {
            out.push(name);
            npm_done = true;
        }
        for f in ["deno.json", "deno.jsonc", "jsr.json"] {
            let Some(m) = read_jsonc(&d.join(f)) else {
                continue;
            };
            out.extend(json_name(&m));
            if let Some(map) = m.get("imports").and_then(|v| v.as_object()) {
                for (k, v) in map {
                    if v.as_str().is_some_and(is_local_target) {
                        out.push(k.clone());
                    }
                }
            }
        }
        if !ts_done {
            for f in ["tsconfig.json", "jsconfig.json"] {
                let p = d.join(f);
                if p.is_file() {
                    ts_done = true;
                    tsconfig_paths(&p, 0, &mut out);
                }
            }
        }
        dir = d.parent();
    }
    out.sort();
    out.dedup();
    out
}

/// `compilerOptions.paths` keys of one tsconfig and its relative
/// `extends` chain (bounded: a config chain can be cyclic).
fn tsconfig_paths(p: &Path, depth: u8, out: &mut Vec<String>) {
    if depth > 4 {
        return;
    }
    let Some(cfg) = read_jsonc(p) else {
        return;
    };
    if let Some(paths) = cfg
        .get("compilerOptions")
        .and_then(|c| c.get("paths"))
        .and_then(|v| v.as_object())
    {
        out.extend(paths.keys().cloned());
    }
    let parents: Vec<&str> = match cfg.get("extends") {
        Some(serde_json::Value::String(s)) => vec![s.as_str()],
        Some(serde_json::Value::Array(a)) => {
            a.iter().filter_map(|v| v.as_str()).collect()
        }
        _ => Vec::new(),
    };
    for parent in parents.into_iter().filter(|e| is_local_target(e)) {
        if let Some(base) = p.parent() {
            // `"extends": "./tsconfig.base"` names `tsconfig.base.json`.
            let named = base.join(parent);
            let next = if named.is_file() {
                named
            } else {
                let mut with_ext = named.into_os_string();
                with_ext.push(".json");
                with_ext.into()
            };
            tsconfig_paths(&next, depth + 1, out);
        }
    }
}

fn is_local_target(s: &str) -> bool {
    s.starts_with("./") || s.starts_with("../") || s == "." || s == ".."
}

fn json_name(m: &serde_json::Value) -> Option<String> {
    m.get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

/// A JSON manifest, tolerating the comments and trailing commas that
/// `tsconfig.json` and `deno.jsonc` allow.
fn read_jsonc(p: &Path) -> Option<serde_json::Value> {
    if !p.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&text)
        .ok()
        .or_else(|| serde_json::from_str(&strip_jsonc(&text)).ok())
}

/// Remove `//` and `/* */` comments outside strings, then any comma that
/// only precedes a closing `}` or `]`.
fn strip_jsonc(text: &str) -> String {
    // One string-aware scan; `keep` decides each byte outside a string.
    fn scan(b: &[u8], mut keep: impl FnMut(&[u8], usize) -> Option<usize>) -> Vec<u8> {
        let mut out = Vec::with_capacity(b.len());
        let (mut i, mut in_str) = (0, false);
        while i < b.len() {
            let c = b[i];
            if in_str {
                out.push(c);
                if c == b'\\' && i + 1 < b.len() {
                    out.push(b[i + 1]);
                    i += 2;
                    continue;
                }
                in_str = c != b'"';
                i += 1;
                continue;
            }
            match keep(b, i) {
                // Skip to this index, dropping what lies between.
                Some(next) => i = next,
                None => {
                    in_str = c == b'"';
                    out.push(c);
                    i += 1;
                }
            }
        }
        out
    }
    let no_comments = scan(text.as_bytes(), |b, i| match (b[i], b.get(i + 1)) {
        (b'/', Some(b'/')) => Some(
            b[i..]
                .iter()
                .position(|&x| x == b'\n')
                .map_or(b.len(), |p| i + p),
        ),
        (b'/', Some(b'*')) => Some(
            b[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(b.len(), |p| i + 2 + p + 2),
        ),
        _ => None,
    });
    let no_trailing = scan(&no_comments, |b, i| {
        let next = b[i + 1..].iter().find(|x| !x.is_ascii_whitespace());
        (b[i] == b',' && matches!(next, Some(b'}') | Some(b']'))).then_some(i + 1)
    });
    String::from_utf8(no_trailing).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_mod_module_and_local_replaces() {
        let text = "module example.com/app // the app\n\ngo 1.21\n\n\
            require github.com/google/uuid v1.6.0\n\
            replace example.com/lib => ../lib\n\
            replace github.com/x/y v1.0.0 => github.com/fork/y v1.0.1\n\
            replace (\n\
            \texample.com/tools v0.1.0 => ./tools\n\
            \tgithub.com/a/b => github.com/c/b v2.0.0\n\
            )\n";
        assert_eq!(
            parse_go_mod(text),
            vec![
                "example.com/app".to_string(),
                "example.com/lib".to_string(),
                "example.com/tools".to_string(),
            ]
        );
    }

    #[test]
    fn go_mod_quoted_module_and_no_false_prefix() {
        assert_eq!(
            parse_go_mod("module \"a.b/c\"\n"),
            vec!["a.b/c".to_string()]
        );
        // `modules` is not the `module` directive.
        assert!(parse_go_mod("modules x\n").is_empty());
    }

    #[test]
    fn js_names_from_package_deno_and_tsconfig() {
        let tmp = std::env::temp_dir()
            .join(format!("__cgg_manifest_js_{}", std::process::id()));
        let sub = tmp.join("pkg").join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            tmp.join("pkg/package.json"),
            r#"{"name": "@acme/core", "author": {"name": "x"}}"#,
        )
        .unwrap();
        // A nameless nested manifest does not end the walk.
        std::fs::write(sub.join("package.json"), r#"{"type": "module"}"#).unwrap();
        std::fs::write(
            tmp.join("deno.json"),
            r#"{"name": "@std/assert", "imports": {"@/": "./src/", "fmt": "jsr:@std/fmt"}}"#,
        )
        .unwrap();
        std::fs::write(
            tmp.join("pkg/tsconfig.json"),
            "{\n  // aliases\n  \"extends\": \"./tsconfig.base\",\n  \"compilerOptions\": {\"paths\": {\"@userActions/*\": [\"./src/libs/actions/*\"],}},\n}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("pkg/tsconfig.base.json"),
            r#"{"compilerOptions": {"paths": {"@shared/*": ["../shared/*"]}}}"#,
        )
        .unwrap();
        let names = js_module_names(&sub.join("index.ts"));
        std::fs::remove_dir_all(&tmp).ok();
        for want in [
            "@acme/core",
            "@std/assert",
            "@/",
            "@userActions/*",
            "@shared/*",
        ] {
            assert!(
                names.iter().any(|n| n == want),
                "{want} missing from {names:?}"
            );
        }
        assert!(
            !names.iter().any(|n| n == "fmt"),
            "a remote import is not local"
        );
    }

    #[test]
    fn jsonc_comments_and_trailing_commas() {
        let v: serde_json::Value = serde_json::from_str(&strip_jsonc(
            "{\"a\": \"x // not a comment\", /* c */ \"b\": [1, 2,], // tail\n}",
        ))
        .unwrap();
        assert_eq!(v["a"], "x // not a comment");
        assert_eq!(v["b"].as_array().unwrap().len(), 2);
    }
}
