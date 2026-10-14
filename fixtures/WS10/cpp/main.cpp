#include "foo.h"
void run(Foo f, void *o, Foo *p) {
  f.m();
  (f).m();
  Foo().m();
  Foo{}.m();
  ((Foo*)o)->m();
  static_cast<Foo*>(o)->m();
  (*p).m();
  p->m();
}
