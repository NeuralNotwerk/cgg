#pragma once
#include <cstdio>
#include "processor.h"
struct Epsilon : Processor {
    void process() override;
    void unusedOp() override;
};
