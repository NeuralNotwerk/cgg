class Tool { fun work() {} }
class Other { fun work() {} }
fun make(): Tool = Tool()
fun other(): Other { return Other() }
fun use() {
    val t = make()
    t.work()
    make().work()
    val o = other()
    o.work()
}
fun a() { val x = make(); x.work() }
fun b() { val x = other(); x.work() }
