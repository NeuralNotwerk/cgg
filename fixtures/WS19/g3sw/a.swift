func outer() {
    func go() {}
    go()
}
func other() {
    let f = { (go: () -> Void) in go() }
    f({})
}
