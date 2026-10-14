class Engine {
  void run() {
    helper();
    tick(1);
  }
  int tick(int x) {
    return x + 1;
  }
}
void helper() {}
