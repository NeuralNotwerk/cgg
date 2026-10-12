package acme

import acme.util.Helpers

object App2 {
  def run2(c: Codec, codec: Any): Unit = {
    Helpers.fromFile("x")
    c.decode("x")
    val k = new Codec()
    k.decode("y")
    codec.decode("z")
  }
}
