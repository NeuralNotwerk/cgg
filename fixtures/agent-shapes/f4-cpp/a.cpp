struct Foo { int zorkle() { return 1; } };
struct Bar { int zorkle() { return 2; } };
struct Repo { int quux() { return 1; } };
struct Other { int quux() { return 2; } };
struct A {
    Repo repo;
    void ctlLocal() { Foo f; f.zorkle(); }
    void multiLocal() {
        Foo f;
        f
            .zorkle();
    }
    void parenLocal() { Foo f; (f).zorkle(); }
    void tempRecv() { Foo().zorkle(); }
    void braceRecv() { Foo{}.zorkle(); }
    void ctlField() { this->repo.quux(); }
    void multiField() {
        this->repo
            .quux();
    }
};
