function go() {
  var api = new JHAPI("/");
  api.stop("x");
  var r = new FileReader();
  r.stop();
}
