package com.acme.app
import com.acme.util.shout
class Box { fun total(): Int = 1 }
class UseIt {
    private fun List<String>.firstUpper(): String = ""
    fun go(items: List<String>, b: Box) {
        items.map { it }.firstUpper()      // same-file member extension
        items.first().shout()              // imported extension
        b.total()                          // typed: member wins
        Box().total()                      // constructed: member wins
        listOf(1).total()                  // not imported, other package: duck as before
    }
}
