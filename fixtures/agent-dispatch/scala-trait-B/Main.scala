object Main {
  def run[T <: Processor](p: T): Unit = {
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
