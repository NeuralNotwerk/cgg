#pragma once
#include <cstdio>
#include "processor.h"
struct Zeta : Processor {
    void process() override;
    void unusedOp() override;
};
