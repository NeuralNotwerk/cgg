#pragma once
#include <cstdio>
#include "processor.hpp"
struct Delta : Processor {
    void process() override;
    void unusedOp() override;
};
