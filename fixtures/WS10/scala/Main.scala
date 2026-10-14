object Main {
  def ceil(x: Double): Double = x
  def run(o: Any, f: Foo): Unit = {
    val a = f.m()
    val b = (f).m()
    val c = new Foo().m()
    val d = (new Foo()).m()
    val e = o.asInstanceOf[Foo].m()
    val g = math.ceil(1.0)
  }
}
