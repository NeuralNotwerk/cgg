fun outer() {
    fun go() {}
    go()
}
fun other() {
    val f = { go: () -> Unit -> go() }
    f {}
}
