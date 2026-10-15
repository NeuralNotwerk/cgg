fn main() {
    let cfg = kb_common::ConfigSource::<kb_common::KbConfig>::load("x").unwrap();
    let a = cfg.get();
    let copy = cfg.clone();
    let b = copy.get();
}
fn with_param(c: &kb_common::ConfigSource<kb_common::KbConfig>) {
    c.get();
}
