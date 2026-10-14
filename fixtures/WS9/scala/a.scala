object O {
  def genF[T](x: T): T = x
  class B { def m[T](x: T): T = x; def plain(): Unit = () }
  def caller(): Unit = {
    val b = new B()
    genF[Int](1)
    b.m[Int](2)
    b.plain()
    genF(3)
  }
}
