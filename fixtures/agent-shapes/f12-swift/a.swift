struct Wrap<T> {
    static func genS() -> Int { return 1 }
    static func ctlS2() -> Int { return 2 }
}
struct Plain {
    static func ctlS() -> Int { return 1 }
}
func useGen() {
    _ = Wrap<Int>.genS()
}
func useCtl() {
    _ = Plain.ctlS()
    _ = Wrap.ctlS2()
}
