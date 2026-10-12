class Foo { int zorkle() => 1; }
class Bar { int zorkle() => 2; }
class Repo { int quux() => 1; }
class Other { int quux() => 2; }
Future<Foo> getFoo() async => Foo();
class A {
  Repo repo = Repo();
  void ctlLocal() { Foo f = Foo(); f.zorkle(); }
  void ctlLocalVar() { var f = Foo(); f.zorkle(); }
  void multiLocal() {
    var f = Foo();
    f
        .zorkle();
  }
  void parenLocal() { var f = Foo(); (f).zorkle(); }
  void ctorRecv() { Foo().zorkle(); }
  void ctlField() { this.repo.quux(); }
  void multiField() {
    this.repo
        .quux();
  }
  Future<void> awaited() async { (await getFoo()).zorkle(); }
}
