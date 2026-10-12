package p
class Foo { fun zorkle(): Int = 1 }
class Bar { fun zorkle(): Int = 2 }
class Repo { fun quux(): Int = 1 }
class Other { fun quux(): Int = 2 }
class A {
    private val repo: Repo = Repo()
    var o: Any? = null
    fun ctlLocal() { val f: Foo = Foo(); f.zorkle() }
    fun ctlLocalInf() { val f = Foo(); f.zorkle() }
    fun multiLocal() {
        val f = Foo()
        f
            .zorkle()
    }
    fun parenLocal() { val f = Foo(); (f).zorkle() }
    fun ctorRecv() { Foo().zorkle() }
    fun castAs() { (o as Foo).zorkle() }
    fun ctlField() { this.repo.quux() }
    fun ctlFieldBare() { repo.quux() }
    fun multiField() {
        this.repo
            .quux()
    }
}
