class Base { Base(int x) { } }
class Box extends Base {
  Box() { this(1); }
  Box(int y) { super(y); }
}
class Main { void run() { Box b = new Box(); } }
