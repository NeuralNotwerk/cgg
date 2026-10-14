use std::env;
fn dump(n: tree_sitter::Node, src: &[u8], depth: usize, field: Option<&str>) {
    let t = n.utf8_text(src).unwrap_or("");
    let t: String = t.chars().take(50).collect::<String>().replace('\n', "\\n");
    println!("{}{}{} [{}-{}] {:?}", "  ".repeat(depth), field.map(|f| format!("{f}: ")).unwrap_or_default(), n.kind(), n.start_position().row + 1, n.end_position().row + 1, if n.child_count() == 0 || t.len() < 40 { t } else { String::new() });
    let mut c = n.walk();
    for (i, ch) in n.children(&mut c).enumerate() {
        if !ch.is_named() && ch.child_count() == 0 && ch.kind().len() < 3 { continue; }
        dump(ch, src, depth + 1, n.field_name_for_child(i as u32));
    }
}
fn main() {
    let a: Vec<String> = env::args().collect();
    let reg = cgg_lang::PluginRegistry::with_v1_plugins();
    let p = reg.by_id(&a[1]).expect("lang");
    let src = std::fs::read(&a[2]).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&p.ts_language_for(std::path::Path::new(&a[2]))).unwrap();
    let tree = parser.parse(&src, None).unwrap();
    dump(tree.root_node(), &src, 0, None);
}
