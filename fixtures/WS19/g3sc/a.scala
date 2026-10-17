object A {
  def outer(): Unit = {
    def go(): Unit = {}
    go()
  }
  def other(): Unit = {
    val f = (go: () => Unit) => go()
    f(() => ())
  }
}
