interface Shape { fun area(): Double
  fun twice(): Double { return 2 * area() } }
abstract class Base { abstract fun perim(): Double }
class Sq : Base(), Shape { override fun area(): Double = 1.0
  override fun perim(): Double = 4.0 }
class Ci : Base(), Shape { override fun area(): Double = 3.0
  override fun perim(): Double = 6.0 }
fun run() { holder.item.area(); holder.item.perim() }
