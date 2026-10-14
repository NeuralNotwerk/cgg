use tree_sitter::Parser;
fn show(lang: tree_sitter::Language, src: &str) {
    let mut p = Parser::new();
    p.set_language(&lang).unwrap();
    let t = p.parse(src, None).unwrap();
    println!("{}\n", t.root_node().to_sexp());
}
#[test]
fn sexp() {
    let which = std::env::var("WS11_LANG").unwrap_or_default();
    let src = std::fs::read_to_string(std::env::var("WS11_SRC").unwrap()).unwrap();
    match which.as_str() {
        "dart" => show(tree_sitter_dart::LANGUAGE.into(), &src),
        "scala" => show(tree_sitter_scala::LANGUAGE.into(), &src),
        "csharp" => show(tree_sitter_c_sharp::LANGUAGE.into(), &src),
        _ => {}
    }
}
