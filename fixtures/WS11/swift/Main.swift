func aControl(x: Foo) {
    x.run()
}

func bOptional(x: Foo?) {
    x?.run()
}

func cIUO(x: Foo!) {
    x.run()
}

func dWrapped(x: Optional<Foo>) {
    x?.run()
}

func mk() -> Foo? {
    return nil
}

func eRet() {
    let foo = mk()
    foo?.run()
}

func fLocal() {
    let y: Foo? = nil
    y?.run()
}

func gLocalPlain() {
    let y: Foo = Foo()
    y.run()
}

class K {
    var f: Foo?
    var g: Foo
    init(g: Foo) { self.g = g }
    func m() {
        f?.run()
    }
    func n() {
        g.run()
    }
    func o() {
        self.f?.run()
    }
}

func mk2() -> Foo {
    return Foo()
}

func hRetPlain() {
    let foo = mk2()
    foo.run()
}

func iUntyped(x) {
    x.run()
}
