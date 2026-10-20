package p

class A {
    def handler = this.&fieldHandler
    private def fieldHandler() { 1 }
    private def m(x) { x }
    private def orphan() { 1 }
    def run(List xs, B obj) {
        def f = this.&m
        xs.each(Util.&helper)
        def g = obj.&go
        return f
    }
    static void main(String[] args) { new A().run([], new B()) }
}
