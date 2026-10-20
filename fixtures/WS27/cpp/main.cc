#include <functional>
#include <map>
#include <vector>

int cb(int x);
int handler(int x);
int item(int x);
int first(int x);
int ok(int x);
int err(int x);
int fn(int x);
int later(int x);
int now();

struct Result {
  int (*ok)(int);
};
std::map<int, std::function<int(int)>> table();
Result make();
std::function<int(int)> pick();
void pass(std::function<int(int)> f);

// Parameter.
int with_param(std::function<int(int)> cb) { return cb(1); }

// Lambda parameter and init-capture.
int with_lambda() {
  auto l = [handler = pick()](std::function<int(int)> item) { return handler(item(2)); };
  return l(nullptr);
}

// Range-for over structured bindings.
int with_range() {
  int t = 0;
  for (auto& [first, fn] : table()) t += fn(first);
  return t;
}

// if initializer.
int with_if() {
  if (auto ok = pick(); ok) return ok(3);
  return 0;
}

// catch parameter.
int with_catch() {
  try { return 0; } catch (std::function<int(int)> err) { return err(4); }
}

// Value position through a parameter: no reference.
void pass_param(std::function<int(int)> later) { pass(later); }

// Positive controls: the initializer sees the function; a call after the
// block reaches it again.
int controls() {
  int n = now();
  { auto later = pick(); later(5); }
  return n + later(6);
}

int main() {
  return with_param(nullptr) + with_lambda() + with_range() + with_if() + with_catch() + controls();
}
