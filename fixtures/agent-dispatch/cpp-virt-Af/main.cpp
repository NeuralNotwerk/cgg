#include "processor.h"
#include "alpha.h"
#include "beta.h"

struct Holder {
    Processor* field;
    void runField() { field->process(); }
};

int main() {
    Alpha o0;
    Beta o1;
    Holder h0{&o0};
    h0.runField();
    Holder h1{&o1};
    h1.runField();
    return 0;
}
