class Box {
  Box(int x) { }
  Box() { this(1) }
}
class Main {
  static void run() { def b = new Box(1) }
}
