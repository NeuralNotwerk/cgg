package p
class Box {
  def genM[T](x: T): T = x
  def ctlM(x: Int): Int = x
}
object U {
  def genF[T](x: T): T = x
  def ctlF(x: Int): Int = x
  def useGen(): Unit = {
    val b = new Box()
    genF[Int](1)
    b.genM[String]("a")
  }
  def useCtl(): Unit = {
    val b = new Box()
    ctlF(1)
    b.ctlM(2)
  }
}
