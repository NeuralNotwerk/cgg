struct Holder {
    let field: Processor
    func runField() { field.process() }
}

func main() {
    Holder(field: Alpha()).runField()
    Holder(field: Beta()).runField()
}

main()
