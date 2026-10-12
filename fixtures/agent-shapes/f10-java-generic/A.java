package p;
import java.util.Map;
class Box<T> { int zorkle() { return 1; } }
class Crate { int zorkle() { return 2; } }
class Repo<K, V> { int quux() { return 1; } }
class Other { int quux() { return 2; } }
public class A {
    void ctlParam(Crate c) { c.zorkle(); }
    void untyped(Object o) { Object b = o; b.zorkle(); }
    void genParam(Box<String> b) { b.zorkle(); }
    void genParam2(Repo<String, Integer> r) { r.quux(); }
    void genParamNested(Box<Map<String, Integer>> b) { b.zorkle(); }
}
