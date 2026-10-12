namespace P {
class Repo { public int Quux() { return 1; } }
class Other { public int Quux() { return 2; } }
public class A {
    void CtlLocal() { Repo r = new Repo(); r.Quux(); }
    void MultiRecv() {
        Repo r = new Repo();
        r
            .Quux();
    }
}
}
