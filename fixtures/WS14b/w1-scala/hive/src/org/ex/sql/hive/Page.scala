package org.ex.sql.hive

import org.json4s.jackson.JsonMethods.{compact, render}
import org.ex.sql.QueryTest

class Page {
  def show(): String = compact(render(1))
}

