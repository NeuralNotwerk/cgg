namespace P {
class Box<T> { public int Zorkle() { return 1; } }
class Crate { public int Zorkle() { return 2; } }
public class A {
    void CtlParam(Crate c) { c.Zorkle(); }
    void GenParam(Box<string> b) { b.Zorkle(); }
    void GenLocal() { Box<string> b = new Box<string>(); b.Zorkle(); }
}
}
