package com.acme.model

open class Base {
    open fun greet() { }
}
class Widget {
    fun paint() { }
    fun close() { }
}
object Registry {
    fun lookup(k: String): Widget = Widget()
}
