class K {
  K(this.x) { init(); }
  K.named() : x = 1 { init(); }
  factory K.make() { return K(2); }
  int get value => compute();
  set value(int v) { store(v); }
  K operator +(K o) { return K(add(o)); }
  static int s() { return helper(); }
  void init() {}
  int compute() => 1;
  void store(int v) {}
  int add(K o) => 1;
  int x;
}
int helper() => 1;
