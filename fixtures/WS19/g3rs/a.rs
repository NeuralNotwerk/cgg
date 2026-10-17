fn outer() {
    fn go() {}
    go();
}
fn other() {
    let f = |go: fn()| go();
    f(outer);
}
