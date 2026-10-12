package p;
class Other { void zork() { } }
class A {
    Other o;
    void zork() { }
    void run(A other) {
        other.o.zork();
        this.o.zork();
        other.self().zork();
        new A().zork();
    }
    A self() { return this; }
}
