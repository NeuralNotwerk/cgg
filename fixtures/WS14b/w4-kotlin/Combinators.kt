package ex

fun <T> wrap(codec: Codec<T>): Codec<T> = object : Codec<T> {
    override fun decode(): T = codec.run { decode() }
    override fun encode(v: T): String = with(codec) { encode(v) }
}

fun useBox(b: Box): Int {
    val x = b.apply { open() }
    return b.let { it.open() } + x.open()
}

fun decode(): Int = 0

fun plain(): Int = run { decode() }

fun helperTop(): Int = 3

class User(val b: Box) {
    fun member(): Int = 4
    fun go(): Int = b.run { open() + helperTop() + member() }
}
