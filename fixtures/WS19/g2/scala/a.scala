object A {
  def outer(): Unit = {
    def helper(): Unit = {}
    helper()
  }
  def other(): Unit = {
    helper()
  }
}
