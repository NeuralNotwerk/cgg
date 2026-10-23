package app

fun go() {
    make().work()
    val t = make()
    t.work()
    val m = maybe()
    m?.work()
    val i = inferred()
    i.work()
}
