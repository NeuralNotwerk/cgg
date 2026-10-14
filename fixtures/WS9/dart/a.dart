T f<T>(T x) => x;
class B {
  T m<T>(T x) => x;
  void plain() {}
}
void caller() {
  var b = B();
  f<int>(1);
  b.m<int>(2);
  b.plain();
  f(3);
}
