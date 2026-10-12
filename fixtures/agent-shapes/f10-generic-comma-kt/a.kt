package p
class Handler { fun containsKey(k: String): Boolean = false }
fun lookup(handlers: Map<String, Handler>) { handlers.containsKey("a") }
