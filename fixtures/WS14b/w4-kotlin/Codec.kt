package ex

interface Codec<T> {
    fun decode(): T
    fun encode(v: T): String
}

class Box {
    fun decode(): Int = 1
    fun open(): Int = 2
}
