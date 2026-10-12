package p
class Repo { fun quux(): Int = 1 }
class Other { fun quux(): Int = 2 }
class Svc(private val repo: Repo) {
    fun syncM() { repo.quux() }
    suspend fun suspendM() { repo.quux() }
    fun syncParam(r: Repo) { r.quux() }
    suspend fun suspendParam(r: Repo) { r.quux() }
    fun syncLocal() { val r = Repo(); r.quux() }
    suspend fun suspendLocal() { val r = Repo(); r.quux() }
}
