//! Jupyter notebook (`.ipynb`) source extraction.
//!
//! A notebook is a JSON document with a `cells` array. Each cell has a
//! `cell_type` (we keep only `"code"`) and a `source` field that is
//! either a single string or an array of strings.
//!
//! [`extract_python_source`] concatenates all code cells back into a
//! single Python source buffer, separated by blank lines, so the
//! Python plugin can parse it as ordinary `.py` content.
//!
//! Non-Python kernels (R, Julia, …) are out of scope — we identify the
//! language only by extension, so cgg currently treats every `.ipynb`
//! as Python. If a notebook is genuinely an R / Julia kernel notebook
//! the resulting tree will simply contain parse errors and produce no
//! callables.

use serde_json::Value;

/// Decode `bytes` as a Jupyter notebook and return the concatenated
/// source of all `code` cells. Returns `None` if the bytes aren't
/// valid JSON or don't have a `cells` array.
pub fn extract_python_source(bytes: &[u8]) -> Option<Vec<u8>> {
    let doc: Value = serde_json::from_slice(bytes).ok()?;
    let cells = doc.get("cells")?.as_array()?;

    let mut out = String::new();
    for cell in cells {
        let Some(kind) = cell.get("cell_type").and_then(Value::as_str) else { continue };
        if kind != "code" { continue }
        let Some(source) = cell.get("source") else { continue };

        let cell_text = match source {
            Value::String(s) => s.clone(),
            Value::Array(lines) => lines
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<String>(),
            _ => continue,
        };

        let trimmed = strip_magics(&cell_text);
        if trimmed.trim().is_empty() { continue }

        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&trimmed);
        if !out.ends_with('\n') { out.push('\n'); }
        // Blank line between cells so cell-local top-level statements
        // don't accidentally chain (e.g. dangling `if` continuations).
        out.push('\n');
    }

    Some(out.into_bytes())
}

/// IPython has magics that aren't valid Python: lines starting with `%`
/// or `%%` (cell magics), and shell escapes starting with `!`. We strip
/// those so the rest of the cell parses cleanly. The remaining lines —
/// including normal Python code — are preserved verbatim.
fn strip_magics(cell: &str) -> String {
    cell.lines()
        .map(|line| {
            let stripped = line.trim_start();
            if stripped.starts_with('%') || stripped.starts_with('!') || stripped.starts_with('?') {
                "" // drop the magic
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_code_cells_and_skips_markdown() {
        let nb = br##"{
  "cells": [
    {"cell_type": "markdown", "source": ["# header\n"]},
    {"cell_type": "code", "source": ["def greet():\n", "    return 1\n"]},
    {"cell_type": "code", "source": "print(greet())\n"}
  ],
  "metadata": {},
  "nbformat": 4
}"##;
        let out = extract_python_source(nb).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("def greet()"), "got: {s}");
        assert!(s.contains("print(greet())"), "got: {s}");
        assert!(!s.contains("# header"), "markdown leaked: {s}");
    }

    #[test]
    fn strips_ipython_magics() {
        let nb = br##"{
  "cells": [
    {"cell_type": "code", "source": ["%matplotlib inline\n", "!pip install foo\n", "import os\n", "def f(): pass\n"]}
  ]
}"##;
        let out = extract_python_source(nb).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("matplotlib"), "magic leaked: {s}");
        assert!(!s.contains("pip install"), "shell escape leaked: {s}");
        assert!(s.contains("import os"));
        assert!(s.contains("def f()"));
    }

    #[test]
    fn returns_none_for_garbage() {
        assert!(extract_python_source(b"not json").is_none());
        assert!(extract_python_source(b"{}").is_none());
    }

    #[test]
    fn empty_cells_array_yields_empty_source() {
        let out = extract_python_source(br##"{"cells": []}"##).unwrap();
        assert!(out.is_empty());
    }
}
