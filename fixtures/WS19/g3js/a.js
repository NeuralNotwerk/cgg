function outer() {
  function go() {}
  return go;
}
function other() {
  const f = (go) => go();
  return f;
}
