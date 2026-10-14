package m

fun aControl(x: Foo) {
    x.run()
}

fun bNullable(x: Foo?) {
    x?.run()
}

fun cNullableBang(x: Foo?) {
    x!!.run()
}

fun dUntypedLambda(xs: List<Foo>) {
    xs.forEach { it.run() }
}

fun mk(): Foo? = null

fun eRet() {
    val foo = mk()
    foo?.run()
}

fun fLocal() {
    val y: Foo? = null
    y?.run()
}

fun gLocalPlain() {
    val y: Foo = Foo()
    y.run()
}

class K(private val f: Foo?, val g: Foo) {
    private var h: Foo? = null
    fun m() {
        f?.run()
    }
    fun n() {
        g.run()
    }
    fun o() {
        h?.run()
    }
}

fun hGeneric(x: List<Foo>?) {
    x?.size
}

fun mk3(): Foo = Foo()

fun iRetPlain() {
    val foo = mk3()
    foo.run()
}

class K2 {
    private var h: Foo = Foo()
    fun o() {
        h.run()
    }
}
