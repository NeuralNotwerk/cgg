class Main {
    // Methods whose names the parameters and locals below reuse.
    def handler(x) { x }
    def cb(x) { x }
    def fn(x) { x }
    def item(x) { x }
    def it(x) { x }
    def ok(x) { x }
    def step(x) { x }
    def err(x) { x }
    def first(x) { x }
    def later(x) { x }
    def now() { 0 }
    def steps() { [] }
    def pair() { [1, 2] }

    // Parameter: an untyped one too.
    def withParam(Closure cb, fn) { cb(1); fn(2) }

    // Closure parameter and the implicit `it`.
    def withClosure() { [1].each { item -> item(3) } }
    def withIt() { [1].each { it(4) } }

    // A `def` local holding a closure.
    def withLocal() {
        def ok = { x -> x }
        ok(5)
    }

    // A `for` variable, a `catch` parameter, multiple assignment.
    def withFor() { for (step in steps()) { step(6) } }
    def withCatch() {
        try { now() } catch (Exception err) { err(7) }
    }
    def withMulti() {
        def (first, second) = pair()
        first(8)
    }

    // A same-class method through a parameter of that name.
    def withHandler(handler) { handler(9) }

    // Positive controls: the initializer sees the function; a call after
    // the block that declares a local reaches the function again.
    def controls() {
        def n = now()
        if (n) {
            def later = { x -> x }
            later(10)
        }
        later(11)
    }
}
