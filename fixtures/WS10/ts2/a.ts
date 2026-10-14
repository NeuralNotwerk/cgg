class Foo { m() {} }
function run() {
  new Foo().m();
  (new Foo()).m();
}
