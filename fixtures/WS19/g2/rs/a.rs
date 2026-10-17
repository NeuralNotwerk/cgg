fn outer() {
    fn helper() {}
    helper();
}
fn other() {
    helper();
}
