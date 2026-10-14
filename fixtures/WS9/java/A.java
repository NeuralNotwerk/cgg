class A {
  <T> T m(T x) { return x; }
  static <T> T s(T x) { return x; }
  void caller(A a) {
    this.<String>m("a");
    a.<String>m("b");
    A.<Integer>s(1);
    m("c");
  }
}
