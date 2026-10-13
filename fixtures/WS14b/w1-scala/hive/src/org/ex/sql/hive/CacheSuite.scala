package org.ex.sql.hive

import org.ex.sql.QueryTest

class CacheSuite extends QueryTest {
  def run(): Unit = {
    withTable("t")
    checkAnswer(1)
  }
}

class Ctx(val n: Int) {
  def this(s: String) = {
    this(s.length)
  }
  def this() = {
    this("x")
  }
}
