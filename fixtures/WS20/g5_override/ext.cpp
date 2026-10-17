#include <QWidget>

class Ext : public QWidget {
public:
    void hookC(int e) override {}
    void hookC2(int e) final {}
    void unusedC() {}
};

int main() { Ext e; return 0; }
