package p;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
class Foo { int zorkle() { return 1; } }
class Bar { int zorkle() { return 2; } }
public class A {
    void ctlParam(Foo f) { f.zorkle(); }
    void ctlUntyped(Object o) { ((Foo) o).zorkle(); }
    void optGet(Optional<Foo> o) { o.get().zorkle(); }
    void optLocal() { Optional<Foo> o = Optional.of(new Foo()); o.get().zorkle(); }
    void atomicGet(AtomicReference<Foo> r) { r.get().zorkle(); }
    void listIter(List<Foo> xs) { for (Foo x : xs) { x.zorkle(); } }
    void listVarIter(List<Foo> xs) { for (var x : xs) { x.zorkle(); } }
    void listGet(List<Foo> xs) { xs.get(0).zorkle(); }
    void varLocal() { var f = new Foo(); f.zorkle(); }
    void genericLocal() { Box<Foo> b = new Box<>(new Foo()); b.zorkle(); }
}
class Box<T> { T v; Box(T v) { this.v = v; } int zorkle() { return 3; } }
