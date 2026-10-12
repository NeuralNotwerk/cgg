using System;
using System.Collections.Generic;
using System.Threading.Tasks;
namespace P {
class Foo { public int Zorkle() { return 1; } }
class Bar { public int Zorkle() { return 2; } }
public class A {
    void CtlParam(Foo f) { f.Zorkle(); }
    void CtlUntyped(dynamic d) { d.Zorkle(); }
    void Nullable(Foo? f) { f?.Zorkle(); }
    void NullableBang(Foo? f) { f!.Zorkle(); }
    void LazyVal(Lazy<Foo> l) { l.Value.Zorkle(); }
    async Task TaskAwait(Task<Foo> t) { (await t).Zorkle(); }
    async Task TaskAwaitLocal(Task<Foo> t) { var x = await t; x.Zorkle(); }
    void ListIter(List<Foo> xs) { foreach (var x in xs) { x.Zorkle(); } }
    void ListIterTyped(List<Foo> xs) { foreach (Foo x in xs) { x.Zorkle(); } }
    void VarLocal() { var f = new Foo(); f.Zorkle(); }
}
}
