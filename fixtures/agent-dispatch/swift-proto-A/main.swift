func run(_ p: Processor) {
    p.process()
}

func main() {
    run(Alpha())
    run(Beta())
}

main()
