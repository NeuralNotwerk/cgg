DEF_FREE(widget)
namespace ns {
class Box {
 public:
  DEF_METHOD(width)
  int real_method() const { return 3; }
  DEF_METHOD(height);
 private:
  int width_ = 0;
};
DEF_FREE(other)
}
