class Repo { func quux() -> Int { return 1 } }
class Other { func quux() -> Int { return 2 } }
class Svc {
    let repo: Repo = Repo()
    func syncParam(r: Repo) { _ = r.quux() }
    func asyncParam(r: Repo) async { _ = r.quux() }
    func syncLocal() { let r = Repo(); _ = r.quux() }
    func asyncLocal() async { let r = Repo(); _ = r.quux() }
}
