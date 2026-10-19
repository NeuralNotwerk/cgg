class Panel {
    var title: String = ""
    func tap() {}
    func handle() {}
    func unused() {}                    // control: dead
    static func process(_ x: Int) -> Int { x }
    func wire(xs: [Int]) {
        button.addTarget(self, action: #selector(tap), for: .touchUpInside)
        let g = self.handle
        let ys = xs.map(Panel.process)
        _ = (g, ys, title)
    }
}
func transform(_ x: Int) -> Int { x }
func compare(_ a: Int, _ b: Int) -> Bool { a < b }
func title() -> String { "" }           // near-miss: only the property `title` is read
func shadowed() {}                       // near-miss: only a parameter of this name is passed
func orphan() {}                         // control: dead
func use(_ shadowed: () -> Void) { invoke(shadowed) }
func invoke(_ f: () -> Void) { f() }
func main() {
    let xs = [3, 1]
    _ = xs.map(transform)
    _ = xs.sorted(by: compare)
    Panel().wire(xs: xs)
    use({})
}
