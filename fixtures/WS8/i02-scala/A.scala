class B {
  def m(): Int = 1
  def size: Int = 2
}
object Main {
  def go(b: B): Int = {
    b.m()
    b.size
    helper
    helper2()
  }
  def helper: Int = 3
  def helper2(): Int = 4
}
