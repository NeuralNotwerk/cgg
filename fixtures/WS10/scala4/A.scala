package org.apache.spark.sql.execution.benchmark
import org.apache.spark.benchmark.Benchmark
object MiscBenchmark extends SqlBasedBenchmark {
  import spark.implicits._
  def f(): Unit = {
    val df = spark.sparkContext.parallelize(Seq(1))
  }
}
