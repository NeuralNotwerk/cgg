function outer(a) {
  function helper() {}
  helper();
}
function other(b) {
  helper();
}
