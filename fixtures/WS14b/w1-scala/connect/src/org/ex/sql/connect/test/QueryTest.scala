package org.ex.sql.connect.test

trait SQLHelper {
  def withTable(name: String): Unit = {}
}

abstract class QueryTest extends SQLHelper {
  def checkAnswer(x: Int): Unit = {}
}
