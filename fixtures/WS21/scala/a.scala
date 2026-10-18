class Box(x: Int) {
  def this() = this(1)
}
object Main {
  def run(): Unit = { val b = new Box(1); val c = new Box() }
}
