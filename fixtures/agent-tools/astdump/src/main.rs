use tree_sitter::{Node, Parser};
fn dump(n: Node, src: &[u8], depth: usize, field: Option<&str>) {
    let t = n.utf8_text(src).unwrap_or("");
    let t = if t.len() > 60 { format!("{}…", &t[..60]) } else { t.to_string() };
    let f = field.map(|f| format!("{}: ", f)).unwrap_or_default();
    if n.is_named() { println!("{}{}{} {:?}", "  ".repeat(depth), f, n.kind(), t.replace('\n', "\\n")); }
    let mut c = n.walk();
    for (i, ch) in n.children(&mut c).enumerate() {
        let fname = n.field_name_for_child(i as u32);
        dump(ch, src, depth + 1, fname);
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let lang: tree_sitter::Language = match a[1].as_str() {
        "py" => tree_sitter_python::LANGUAGE.into(),
        "js" => tree_sitter_javascript::LANGUAGE.into(),
        "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "cs" => tree_sitter_c_sharp::LANGUAGE.into(),
        "kt" => tree_sitter_kotlin_sg::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "dart" => tree_sitter_dart::LANGUAGE.into(),
        "scala" => tree_sitter_scala::LANGUAGE.into(),
        _ => panic!(),
    };
    let src = std::fs::read(&a[2]).unwrap();
    let mut p = Parser::new(); p.set_language(&lang).unwrap();
    let t = p.parse(&src, None).unwrap();
    dump(t.root_node(), &src, 0, None);
}
