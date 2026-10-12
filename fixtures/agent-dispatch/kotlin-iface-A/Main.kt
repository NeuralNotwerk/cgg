fun run(p: Processor) {
    p.process()
}

fun main() {
    run(Alpha())
    run(Beta())
}
