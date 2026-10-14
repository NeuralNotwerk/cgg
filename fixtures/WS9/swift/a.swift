struct Wrap<T> {
  static func s() {}
  func m() {}
}
func gen<T>(_ x: T) -> T { return x }
func caller() {
  Wrap<Int>.s()
  let w = Wrap<Int>()
  w.m()
  _ = gen(1)
}
