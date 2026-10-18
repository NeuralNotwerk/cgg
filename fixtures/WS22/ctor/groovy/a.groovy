class Base {
  Base(int n) { }
}
class Box extends Base {
  Box(int x) { super(x) }
  Box() { this(1) }
  int size() { 1 }
}
class Plain {
  Plain() { super() }
}
class Main {
  static void run() {
    def b = new Box(1)
    def c = new Box()
    def d = new java.util.ArrayList()
    def e = new Plain()
  }
}
