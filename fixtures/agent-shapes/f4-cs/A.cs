namespace P {
class Foo { public int A; public int Zorkle() { return 1; } }
class Bar { public int A; public int Zorkle() { return 2; } }
class Repo { public int Quux() { return 1; } }
class Other { public int Quux() { return 2; } }
public class A {
    private Repo repo = new Repo();
    object o;
    void CtlLocal() { Foo f = new Foo(); f.Zorkle(); }
    void MultiLocal() {
        Foo f = new Foo();
        f
            .Zorkle();
    }
    void ParenLocal() { Foo f = new Foo(); (f).Zorkle(); }
    void NewRecv() { new Foo().Zorkle(); }
    void ObjInit() { new Foo { A = 1 }.Zorkle(); }
    void CastAs() { (o as Foo).Zorkle(); }
    void CtlField() { this.repo.Quux(); }
    void MultiField() {
        this.repo
            .Quux();
    }
    async System.Threading.Tasks.Task<Foo> GetFoo() { return new Foo(); }
    async System.Threading.Tasks.Task Awaited() { (await GetFoo()).Zorkle(); }
}
}
