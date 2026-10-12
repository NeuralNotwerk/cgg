package acme

import play.api.libs.json.Json
import io.circe.parser

object App {
  def run(): Unit = {
    scala.io.Source.fromFile("x")
    Json.parse("{}")
    parser.decode("x")
  }
}
