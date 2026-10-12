#include "processor.hpp"
#include "alpha.h"
#include "beta.h"
#include "gamma.h"
#include "delta.h"
#include "epsilon.h"
#include "zeta.h"

struct Holder {
    Processor* field;
    void runField() { field->process(); }
};

int main() {
    Alpha o0;
    Beta o1;
    Gamma o2;
    Delta o3;
    Epsilon o4;
    Zeta o5;
    Holder h0{&o0};
    h0.runField();
    Holder h1{&o1};
    h1.runField();
    return 0;
}
