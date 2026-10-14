fun <T> f(x: T): T = x
class B { fun <T> m(x: T): T = x }
fun caller(b: B) {
  f<Int>(1)
  b.m<String>("a")
  f(2)
}
