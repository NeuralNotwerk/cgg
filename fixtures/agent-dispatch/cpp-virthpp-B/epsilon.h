#pragma once
#include <cstdio>
#include "processor.hpp"
struct Epsilon : Processor {
    void process() override;
    void unusedOp() override;
};
