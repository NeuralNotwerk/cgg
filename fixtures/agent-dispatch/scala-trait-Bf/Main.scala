object Main {

  def main(args: Array[String]): Unit = {
    new Holder(new Alpha()).runField()
    new Holder(new Beta()).runField()
  }
}
