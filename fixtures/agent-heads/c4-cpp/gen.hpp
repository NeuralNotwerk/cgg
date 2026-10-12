#define DEF_METHOD(n) int get_##n() const { return n##_; }
#define DEF_FREE(n) inline int make_##n() { return 1; }
DEF_FREE(widget)
class Box {
 public:
  DEF_METHOD(width)
  int real_method() const { return 3; }
 private:
  int width_ = 0;
};
