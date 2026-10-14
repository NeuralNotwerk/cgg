package m

object Main {
  def aControl(x: Foo): Unit = {
    x.run()
  }
  def bNull(x: Foo | Null): Unit = {
    x.run()
  }
  def cMulti(x: Foo | Bar): Unit = {
    x.run()
  }
  def dClosure(xs: List[Foo]): Unit = {
    xs.foreach(x => x.run())
  }
  def eTypedClosure(xs: List[Foo]): Unit = {
    xs.foreach((x: Foo) => x.run())
  }
  def fBlockClosure(xs: List[Foo]): Unit = {
    xs.foreach { x => x.run() }
  }
  def gUntypedPlaceholder(xs: List[Foo]): Unit = {
    xs.foreach(_.run())
  }
  def each(cb: Foo => Unit): Unit = {}
  def hCallback(): Unit = {
    each(x => x.run())
  }
}
