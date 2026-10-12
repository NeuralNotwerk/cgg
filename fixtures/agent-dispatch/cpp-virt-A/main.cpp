#include "processor.h"
#include "alpha.h"
#include "beta.h"

void run(Processor& p) {
    p.process();
}

int main() {
    Alpha o0;
    Beta o1;
    run(o0);
    run(o1);
    return 0;
}
