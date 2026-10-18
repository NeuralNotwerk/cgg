class Base(n: Int) {
  def size(): Int = n
}
class Box(x: Int) extends Base(x) with Serializable {
  def this() = this(1)
  def get(): Int = x
}
class Plain {
  def go(): Unit = ()
}
case class Pt(a: Int, b: Int)
object Main {
  def run(): Unit = {
    val b = new Box(1)
    val c = new Box()
    val p = Pt(1, 2)
    val q = new Plain()
    val anon = new Base(3) { override def size(): Int = 0 }
  }
}
