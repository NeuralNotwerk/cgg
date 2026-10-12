func ctlTyped(x: Foo) { _ = x.zorkle() }
func untyped(x: Any) { _ = x.zorkle() }
func optChain(x: Foo?) { _ = x?.zorkle() }
func optBang(x: Foo?) { _ = x!.zorkle() }
