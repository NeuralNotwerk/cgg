#include <string>
namespace ns { template <typename T> struct A { static void s() {} template <typename U> static U t(U u) { return u; } }; }
template <typename T> struct Base { void f() {} };
template <typename T> struct D : Base<T> { void g() { Base<T>::f(); this->template h<int>(); } template <typename U> void h() {} };
void caller(ns::A<int>* p) {
  ns::A<int>::s();
  ns::A<int>::t<float>(1.0f);
  ns::A<std::string>::s();
  std::make_shared<int>(3);
  std::get<0>(x);
}
