func run<T: Processor>(_ p: T) {
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
