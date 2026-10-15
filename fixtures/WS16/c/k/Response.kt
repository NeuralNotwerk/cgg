package web
class Response(val code: Int) {
  class Builder {
    fun build(): Response = Response(200)
  }
}
