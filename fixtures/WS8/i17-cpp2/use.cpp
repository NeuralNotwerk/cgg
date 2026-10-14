#include "foo.h"
void use(ns::Foo x, const ns::Foo& y) {
  x.m();
  y.m();
}
