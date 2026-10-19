package p

import Funcs._

class Svc(val label: String) {
  val count = 1
  def size: Int = 2                 // parameterless: `size` below is a call, not a value
  def handle(x: Int): Int = x
  def unused(): Unit = ()           // control: dead
  def go(xs: List[Int]): Int = {
    val ys = xs.map(parse)
    val g = handle _
    val h = Util.twice _
    val k = (parse _) andThen inc
    ys.size + size + count + g(1) + h(1) + k(1)
  }
}
object Util { def twice(x: Int): Int = x * 2 }
object Funcs {
  def parse(x: Int): Int = x
  def inc(x: Int): Int = x + 1
  def count(): Int = 0              // near-miss: only the val `count` is read
  def orphan(): Unit = ()           // control: dead
}
object Main {
  import Funcs._
  def main(args: Array[String]): Unit = { new Svc("a").go(List(1)) }
}
