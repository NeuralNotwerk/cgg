func run(o: Any, f: Foo) async {
    let a = f.m()
    let b = (f).m()
    let c = (o as! Foo).m()
    let d = (o as? Foo)?.m()
    let e = Foo().m()
    let g = (Foo()).m()
    let h = f!.m()
}
