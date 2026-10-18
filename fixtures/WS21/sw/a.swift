class Engine {
  init() { }
  init(power: Int) { }
}
class Car: Engine {
  override init() { super.init(power: 1) }
}
func build() {
  let e = Engine()
  let c = Car()
}
