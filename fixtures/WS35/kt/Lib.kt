package app

class Tool { fun work() {} }
class Other { fun work() {} }
fun make(): Tool = Tool()
fun many(): List<Tool> = listOf()
fun maybe(): Tool? = null
fun inferred() = Other()
fun <T> pick(x: T): T = x
