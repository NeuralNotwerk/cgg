class Svc {
    def field = 1
    String name
    Svc(Closure init) { init() }
    def run(Closure cb, String s = "x", Object... rest) {
        cb(s)
        def loc = { a, b -> a(b) }
        String typed = "t"
        def (p, q) = [1, 2]
        for (x in items) { x() }
        for (int i = 0; i < 3; i++) { i() }
        for (String y : items) { y() }
        try { go() } catch (IOException e) { e() }
        items.each { it() }
        items.each { v -> v() }
        def m = it
    }
}
def scriptLocal = { -> 1 }
scriptLocal()
