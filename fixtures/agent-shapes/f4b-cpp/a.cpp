struct Repo { int quux() { return 1; } };
struct Other { int quux() { return 2; } };
struct Inner { Repo repo; };
struct A {
    Repo repo;
    Inner inner;
    void ctlField() { this->repo.quux(); }
    void multiRecv() {
        this
            ->repo
            .quux();
    }
    void ctlTwo() { inner.repo.quux(); }
    void multiTwo() {
        inner
            .repo
            .quux();
    }
};
