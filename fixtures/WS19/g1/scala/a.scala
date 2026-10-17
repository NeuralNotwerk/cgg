class Service {
  def run(): Unit = {}
}
object M {
  def main(): Unit = {
    val service = mk()
    service.run()
  }
}
