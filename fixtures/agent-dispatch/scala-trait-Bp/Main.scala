object Main {
  def run(p: Processor): Unit = {
    p.process()
  }

  def main(args: Array[String]): Unit = {
    run(new Alpha())
    run(new Beta())
    run(new Gamma())
    run(new Delta())
    run(new Epsilon())
    run(new Zeta())
  }
}
