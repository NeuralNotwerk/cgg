package com.acme.util

fun decodeFromString(s: String): String = s
fun runBlocking(block: () -> Unit) { block() }
class Repo {
    fun readText(p: String): String = p
    fun encodeToString(o: Any): String = ""
}
