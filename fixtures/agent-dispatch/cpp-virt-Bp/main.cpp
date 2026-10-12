#include "processor.h"
#include "alpha.h"
#include "beta.h"
#include "gamma.h"
#include "delta.h"
#include "epsilon.h"
#include "zeta.h"

void run(Processor& p) {
    p.process();
}

int main() {
    Alpha o0;
    Beta o1;
    Gamma o2;
    Delta o3;
    Epsilon o4;
    Zeta o5;
    run(o0);
    run(o1);
    run(o2);
    run(o3);
    run(o4);
    run(o5);
    return 0;
}
