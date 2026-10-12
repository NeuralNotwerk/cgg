#pragma once
#include <cstdio>
#include "processor.hpp"
struct Gamma : Processor {
    void process() override;
    void unusedOp() override;
};
