#define ACCESSOR(n) int n() const { return n##_; } void set_##n(int v) { n##_ = v; touch(); }
#define FREE_FN(n) inline int n##_twice(int x) { return x * 2; }
namespace geo {
FREE_FN(len)
class Rect {
 public:
  ACCESSOR(w)
  ACCESSOR(h);
  void touch() {}
 private:
  int w_ = 0, h_ = 0;
};
int area(const Rect &r) { return r.w() * r.h() + len_twice(1); }
}
int main() { geo::Rect r; r.set_w(2); return geo::area(r); }
