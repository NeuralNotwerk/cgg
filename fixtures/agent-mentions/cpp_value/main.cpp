#include <algorithm>
#include <thread>
#include <vector>
#include <functional>

namespace {
bool cmp_fn(int a, int b) { return a < b; }
void thread_fn() {}
int fn_assigned() { return 1; }
void dead_control() {}
void called_control() {}
}

class Widget {
public:
    void run();
private:
    void member_target() {}
    void bound_target() {}
    void priv_dead() {}
};

void Widget::run() {
    auto f = std::bind(&Widget::bound_target, this);
    std::function<void()> g = [this] { member_target(); };
    f(); g();
}

static void static_cb() {}

int main() {
    std::vector<int> v{3, 1, 2};
    std::sort(v.begin(), v.end(), cmp_fn);
    std::thread t(thread_fn);
    t.join();
    std::function<int()> h = fn_assigned;
    void (*p)() = &static_cb;
    called_control();
    Widget w; w.run();
    return h();
}
