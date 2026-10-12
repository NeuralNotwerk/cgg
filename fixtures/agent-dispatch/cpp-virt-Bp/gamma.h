#pragma once
#include <cstdio>
#include "processor.h"
struct Gamma : Processor {
    void process() override;
    void unusedOp() override;
};
