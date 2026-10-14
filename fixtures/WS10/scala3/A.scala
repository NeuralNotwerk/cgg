package app.core
import org.apache.commons.text.StringEscapeUtils
import app.util.Helper
class A {
  val io = new Helper
  def run(h: Helper): Unit = {
    math.ceil(1.0)
    StringEscapeUtils.escapeJava("x")
    io.ceil(2.0)
  }
  def param(math: Helper): Unit = { math.ceil(3.0) }
}
