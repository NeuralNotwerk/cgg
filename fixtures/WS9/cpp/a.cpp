namespace ns {
template <typename T> T nf(T x) { return x; }
}
template <typename T> T f(T x) { return x; }
struct B {
  template <typename T> T m(T x) { return x; }
  template <typename T> static T s(T x) { return x; }
  void plain() {}
};
void caller() {
  B b;
  f<int>(1);
  b.m<int>(2);
  B::s<int>(3);
  ns::nf<int>(4);
  b.template m<int>(5);
  b.plain();
  f(6);
}
