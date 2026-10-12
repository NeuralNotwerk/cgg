package com.acme

import kotlinx.serialization.json.Json
import java.io.File

fun main() {
    val x = Json.decodeFromString<String>("x")
    val y = Json.encodeToString("x")
    File("x").readText()
    kotlinx.coroutines.runBlocking { }
}
