package com.acme.dates
private fun zork(date: Int, field: Int, amount: Int) { }
class Tracker { fun zork(x: Int) { } }
fun make() = listOf(1)
fun use() { val l = make(); l.zork(1) }     // untyped receiver: Tracker.zork may be it, the top-level `zork` never is
