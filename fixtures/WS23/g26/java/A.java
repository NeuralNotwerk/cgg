class Tool { void work() {} }
class Other { void work() {} }
class Factory {
    public static Tool make() { return new Tool(); }
    private java.util.List<Tool> many() { return null; }
    Other other() { return new Other(); }
    void use() {
        var t = make();
        t.work();
        make().work();
        Tool u = make();
        u.work();
        var o = other();
        o.work();
        many().size();
    }
    // same local name typed differently in two functions
    void a() { var x = make(); x.work(); }
    void b() { var x = other(); x.work(); }
    // reassigned local: Java static typing keeps the declared type
    void c() { var y = make(); y = make(); y.work(); }
}
