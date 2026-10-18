class Tool { void work() {} }
class Other { void work() {} }
class F {
    static Tool make() { return new Tool() }
    Other other() { new Other() }
    def dyn() { new Tool() }
    void use() {
        def t = make()
        t.work()
        make().work()
        def o = other()
        o.work()
        def d = dyn()
        d.work()
    }
}
