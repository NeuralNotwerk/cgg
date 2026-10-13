package org.ex.sql

trait QueryTestBase {
  def withTable(name: String): Unit = {}
}

trait QueryTest extends QueryTestBase {
  def checkAnswer(x: Int): Unit = {}
}

object QueryTest {
  def checkAnswer(x: Int, y: Int): Unit = {}
}
