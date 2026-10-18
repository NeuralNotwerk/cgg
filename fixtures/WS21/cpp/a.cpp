class Base { public: Base(int x) {} };
class D : public Base {
 public:
  D() : Base(1) {}
  D(int y) : D() {}
};
void make() { D d(2); D* p = new D(); }
