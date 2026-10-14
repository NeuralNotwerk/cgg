namespace ns { struct Foo { void m(); }; }
std::string to_string(int x) { return ""; }
void use(ns::Foo x) {
  x.m();
  to_string(1);
}
