#pragma once
#include <cstdio>
#include "processor.hpp"
struct Zeta : Processor {
    void process() override;
    void unusedOp() override;
};
