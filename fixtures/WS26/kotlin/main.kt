package p

class Item(val name: String)
class Other { fun name(): String = "" }          // near-miss: Item::name is a property reference
class Box {
    fun handle(x: Int) {}
    fun unused() {}                               // control: dead
    fun start(xs: List<Int>) { xs.forEach(this::handle) }
}
object Util { fun twice(x: Int) = x * 2 }
fun top(x: Int) = x
fun stored(x: Int) = x
fun inString() {}                                 // near-miss: only a string
fun orphan() {}                                   // control: dead
fun take(f: (Int) -> Int) = f(1)
fun useParam(top: (Int) -> Int) = take(top)       // near-miss: a parameter named like top, no ::
fun main() {
    val xs = listOf(1)
    xs.map(::top)
    xs.map(Util::twice)
    val g = ::stored
    println("inString")
    val items = listOf(Item("a")).sortedBy(Item::name)
    val k = Box::class
    Box().start(xs)
    println(g); println(items); println(k); println(useParam { it })
}
