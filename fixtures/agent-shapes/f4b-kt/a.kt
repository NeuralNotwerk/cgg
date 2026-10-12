package p
class Repo { fun quux(): Int = 1 }
class Other { fun quux(): Int = 2 }
class A {
    val repo: Repo = Repo()
    fun ctlFieldBare() { repo.quux() }
    fun multiRecv() {
        repo
            .quux()
    }
}
