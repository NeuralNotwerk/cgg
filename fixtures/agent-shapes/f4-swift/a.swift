class Foo { func zorkle() -> Int { return 1 } }
class Bar { func zorkle() -> Int { return 2 } }
class Repo { func quux() -> Int { return 1 } }
class Other { func quux() -> Int { return 2 } }
class A {
    let repo: Repo = Repo()
    func ctlLocal() { let f: Foo = Foo(); _ = f.zorkle() }
    func ctlLocalInf() { let f = Foo(); _ = f.zorkle() }
    func multiLocal() {
        let f = Foo()
        _ = f
            .zorkle()
    }
    func parenLocal() { let f = Foo(); _ = (f).zorkle() }
    func ctorRecv() { _ = Foo().zorkle() }
    func ctlField() { _ = self.repo.quux() }
    func multiField() {
        _ = self.repo
            .quux()
    }
}
