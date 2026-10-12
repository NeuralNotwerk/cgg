func run(_ p: Processor) {
    p.process()
}

func main() {
    run(Alpha())
    run(Beta())
    run(Gamma())
    run(Delta())
    run(Epsilon())
    run(Zeta())
}

main()
