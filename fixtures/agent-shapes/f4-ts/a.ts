class Foo { zorkle(): number { return 1; } }
class Bar { zorkle(): number { return 2; } }
class Repo { quux(): number { return 1; } }
class Other { quux(): number { return 2; } }
async function getFoo(): Promise<Foo> { return new Foo(); }
export class A {
  private repo: Repo = new Repo();
  o: unknown;
  ctlLocal() { const f: Foo = new Foo(); f.zorkle(); }
  ctlLocalInf() { const f = new Foo(); f.zorkle(); }
  multiLocal() {
    const f = new Foo();
    f
      .zorkle();
  }
  parenLocal() { const f = new Foo(); (f).zorkle(); }
  newRecv() { new Foo().zorkle(); }
  castAs() { (this.o as Foo).zorkle(); }
  ctlField() { this.repo.quux(); }
  multiField() {
    this.repo
      .quux();
  }
  async awaited() { (await getFoo()).zorkle(); }
}
