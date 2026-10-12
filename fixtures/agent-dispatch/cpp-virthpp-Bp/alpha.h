#pragma once
#include <cstdio>
#include "processor.hpp"
struct Alpha : Processor {
    void process() override;
    void unusedOp() override;
};
