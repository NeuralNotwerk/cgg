#pragma once
#include <cstdio>
#include "processor.h"
struct Alpha : Processor {
    void process() override;
    void unusedOp() override;
};
