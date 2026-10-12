package p
class Box {
    fun <T> genM(x: T): T = x
    fun ctlM(x: Int): Int = x
}
fun <T> genF(x: T): T = x
fun ctlF(x: Int): Int = x
fun useGen() {
    val b = Box()
    genF<Int>(1)
    b.genM<String>("a")
}
fun useCtl() {
    val b = Box()
    ctlF(1)
    b.ctlM(2)
}
