object M {
  def f(xs: List[Foo]): Unit = {
    xs.foreach((x: Foo) => x.run())
    xs.foreach { (x: Foo) => x.run() }
    xs.foreach { case x: Foo => x.run() }
    xs.foreach(x => x.run())
  }
}
