trait Shape {
  def area(): Double
  def describe(): String = "shape"
}

class Square extends Shape {
  def area(): Double = 1
}

class Circle extends Shape {
  override def area(): Double = 3
}

class Other {
  def area(): Double = 99
}

object Main {
  def total(s: Shape): Double = s.area()
}
