class Foo { zorkle() { return 1; } }
class Bar { zorkle() { return 2; } }
class Repo { quux() { return 1; } }
class Other { quux() { return 2; } }
async function getFoo() { return new Foo(); }
export class A {
  repo = new Repo();

  ctlLocal() { const f = new Foo(); f.zorkle(); }
  ctlLocalInf() { const f = new Foo(); f.zorkle(); }
  multiLocal() {
    const f = new Foo();
    f
      .zorkle();
  }
  parenLocal() { const f = new Foo(); (f).zorkle(); }
  newRecv() { new Foo().zorkle(); }
  castAs() { (this.o).zorkle(); }
  ctlField() { this.repo.quux(); }
  multiField() {
    this.repo
      .quux();
  }
  async awaited() { (await getFoo()).zorkle(); }
}
