void outer() {
  void helper() {}
  helper();
}
void other() {
  helper();
}
