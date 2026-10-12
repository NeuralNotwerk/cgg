object Main {
  def run(p: Processor): Unit = {
    p.process()
  }

  def main(args: Array[String]): Unit = {
    run(new Alpha())
    run(new Beta())
  }
}
