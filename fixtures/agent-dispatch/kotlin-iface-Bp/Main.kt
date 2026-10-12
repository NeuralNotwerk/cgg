fun run(p: Processor) {
    p.process()
}

fun main() {
    run(Alpha())
    run(Beta())
    run(Gamma())
    run(Delta())
    run(Epsilon())
    run(Zeta())
}
