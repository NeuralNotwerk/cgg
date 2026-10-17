func outer() {
    func helper() {}
    helper()
}
func other() {
    helper()
}
