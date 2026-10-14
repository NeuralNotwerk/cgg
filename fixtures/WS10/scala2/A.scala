package app.core
import org.apache.commons.text.StringEscapeUtils
import scala.collection.mutable
import app.util.{Helper => H}
class A {
  val io = new Helper
  def run(math: Helper): Unit = {
    StringEscapeUtils.escapeJava("x")
    mutable.Map.empty
    math.ceil(1.0)
    io.ceil(2.0)
    H.ceil(3.0)
    util.Try(1)
  }
}
