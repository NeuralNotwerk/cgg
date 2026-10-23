//! Preprocessor branches of C-family definitions (WS36).
//!
//! `#ifdef _WIN32 … void f() {…} #else … void f() {…} #endif` defines one
//! function twice, once per configuration. Each definition records the
//! branches it sits in ([`cgg_core::DefRecord::preproc_branches`]), so the
//! resolver can tell such alternatives from a real duplicate.

use cgg_core::DefRecord;
use tree_sitter::Node;

/// Node kinds that head a conditional chain.
const CHAIN_HEADS: &[&str] = &["preproc_if", "preproc_ifdef"];

/// One branch of a chain: the chain's start byte, the branch's position
/// in it, and the half-open byte span the branch covers.
struct Branch {
    chain: u32,
    index: u32,
    start: u32,
    end: u32,
}

/// Every branch of every conditional chain under `root`, walked without
/// recursion.
fn branches(root: Node) -> Vec<Branch> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if CHAIN_HEADS.contains(&n.kind()) {
            let chain = n.start_byte() as u32;
            let mut cur = Some(n);
            let mut index = 0u32;
            while let Some(b) = cur {
                let alt = b.child_by_field_name("alternative");
                out.push(Branch {
                    chain,
                    index,
                    start: b.start_byte() as u32,
                    end: alt.map_or(b.end_byte(), |a| a.start_byte()) as u32,
                });
                index += 1;
                cur = alt;
            }
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    out
}

/// Record on each of `defs` the preprocessor branches that hold it,
/// outermost first.
pub(crate) fn mark(root: Node, defs: &mut [DefRecord]) {
    if defs.is_empty() {
        return;
    }
    let mut all = branches(root);
    if all.is_empty() {
        return;
    }
    // Outermost first: a containing branch starts no later and ends no
    // earlier than one inside it.
    all.sort_by_key(|b| (b.start, std::cmp::Reverse(b.end)));
    for d in defs.iter_mut() {
        d.preproc_branches = all
            .iter()
            .filter(|b| b.start <= d.start_byte && d.end_byte <= b.end)
            .map(|b| (b.chain, b.index))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs_of(src: &str) -> Vec<(String, Vec<(u32, u32)>)> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let mut defs: Vec<DefRecord> = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            if n.kind() == "function_definition" {
                defs.push(DefRecord {
                    simple_name: n.utf8_text(src.as_bytes()).unwrap()[..12].to_string(),
                    start_byte: n.start_byte() as u32,
                    end_byte: n.end_byte() as u32,
                    ..Default::default()
                });
            }
            let mut c = n.walk();
            stack.extend(n.children(&mut c));
        }
        mark(tree.root_node(), &mut defs);
        defs.sort_by_key(|d| d.start_byte);
        defs.into_iter()
            .map(|d| (d.simple_name, d.preproc_branches))
            .collect()
    }

    #[test]
    fn branches_of_one_chain_are_alternatives() {
        let got = defs_of(concat!(
            "#ifdef W\nvoid f1(void){}\n#elif defined(M)\nvoid f2(void){}\n",
            "#else\n#if B\nvoid f3(void){}\n#endif\nvoid f4(void){}\n#endif\n",
            "void f5(void){}\n",
        ));
        let b: Vec<&Vec<(u32, u32)>> = got.iter().map(|(_, b)| b).collect();
        assert_eq!(b[0], &vec![(0, 0)]);
        assert_eq!(b[1], &vec![(0, 1)]);
        assert_eq!(b[2].len(), 2);
        assert_eq!(b[2][0], (0, 2));
        assert_eq!(b[2][1].1, 0);
        assert_eq!(b[3], &vec![(0, 2)]);
        assert!(b[4].is_empty());
    }
}
