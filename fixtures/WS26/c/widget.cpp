#include <algorithm>
#include <vector>

namespace ui {
class Widget {
public:
    void onClick() {}
    void unused() {}                 // control: dead
    void wire();
};
bool byName(int a, int b) { return a < b; }
}

void connect(void (ui::Widget::*m)()) {}

void ui::Widget::wire() { connect(&ui::Widget::onClick); }

int main() {
    std::vector<int> v;
    std::sort(v.begin(), v.end(), ui::byName);  // not read: a qualified name without & is an enumerator more often
    ui::Widget w;
    w.wire();
}
