enum M { struct Wrap<T> { static func s() {} } }
struct Wrap<T> { static func s(_ x: Int) {} static func make() -> Wrap<T> { return Wrap<T>() } }
func caller(a: Int, b: Int, c: Int) {
  M.Wrap<Int>.s()
  foo(Wrap<String>.s(1))
  if a < b && c > 3 {}
  let x: Wrap<Int> = .make()
}
func foo(_ x: Any) {}
