fn main() {
    let s = std::fs::read_to_string("x").unwrap();
    std::process::exit(s.len() as i32);
}
