package p
fun ctlTyped(x: Foo) { x.zorkle() }
fun untypedBang(x: Any?) { x!!.zorkle() }
fun nullableParam(x: Foo?) { x?.zorkle() }
fun nullableParamBang(x: Foo?) { x!!.zorkle() }
fun genericParam(x: List<Foo>) { x.first().zorkle() }
