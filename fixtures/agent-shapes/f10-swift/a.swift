class Foo { func zorkle() -> Int { return 1 } }
class Bar { func zorkle() -> Int { return 2 } }
func ctlTyped(x: Foo) { _ = x.zorkle() }
func optChain(x: Foo?) { _ = x?.zorkle() }
func optBang(x: Foo?) { _ = x!.zorkle() }
func iuo(x: Foo!) { _ = x.zorkle() }
func ifLet(x: Foo?) { if let y = x { _ = y.zorkle() } }
func arrIter(xs: [Foo]) { for x in xs { _ = x.zorkle() } }
