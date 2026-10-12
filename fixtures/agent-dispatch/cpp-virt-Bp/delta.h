#pragma once
#include <cstdio>
#include "processor.h"
struct Delta : Processor {
    void process() override;
    void unusedOp() override;
};
