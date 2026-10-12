package p
class Foo { fun zorkle(): Int = 1 }
class Bar { fun zorkle(): Int = 2 }
fun ctlTyped(x: Foo) { x.zorkle() }
fun ctlUntyped(x: Any) { (x as Foo).zorkle() }
fun nullableParam(x: Foo?) { x?.zorkle() }
fun nullableBang(x: Foo?) { x!!.zorkle() }
fun nullableLocal() { val x: Foo? = Foo(); x?.zorkle() }
fun listIter(xs: List<Foo>) { for (x in xs) { x.zorkle() } }
fun lazyVal() { val x by lazy { Foo() }; x.zorkle() }
class Holder {
    lateinit var f: Foo
    var g: Foo? = null
    fun lateinitUse() { f.zorkle() }
    fun nullableField() { g?.zorkle() }
}
