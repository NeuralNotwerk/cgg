package p;
class Repo { int quux() { return 1; } }
class Other { int quux() { return 2; } }
public class A {
    void ctlLocal() { Repo r = new Repo(); r.quux(); }
    void multiRecvLocal() {
        Repo r = new Repo();
        r
          .quux();
    }
}
