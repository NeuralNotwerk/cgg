package docs
object Controller4 {
  object MyActor {
    def props(out: Int): Int = out
  }
  def socket = MyActor.props(1)
}
object Controller5 {
  object MyActor {
    def props(out: Int): Int = out + 1
  }
  def socket = MyActor.props(2)
}
