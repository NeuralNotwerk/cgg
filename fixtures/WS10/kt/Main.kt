fun run(o: Any, f: Foo) {
    val a = f.m()
    val b = (f).m()
    val c = (o as Foo).m()
    val d = (o as? Foo)?.m()
    val e = Foo().m()
    val g = (Foo()).m()
    val h = f!!.m()
}
