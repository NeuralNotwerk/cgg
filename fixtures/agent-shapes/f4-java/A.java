package p;
class Foo { int zorkle() { return 1; } }
class Bar { int zorkle() { return 2; } }
class Repo { int quux() { return 1; } }
class Other { int quux() { return 2; } }
public class A {
    private Repo repo = new Repo();
    Object o;
    void ctlLocal() { Foo f = new Foo(); f.zorkle(); }
    void multiLocal() {
        Foo f = new Foo();
        f
            .zorkle();
    }
    void parenLocal() { Foo f = new Foo(); (f).zorkle(); }
    void newRecv() { new Foo().zorkle(); }
    void castParen() { ((Foo) o).zorkle(); }
    void ctlField() { this.repo.quux(); }
    void multiField() {
        this.repo
            .quux();
    }
}
