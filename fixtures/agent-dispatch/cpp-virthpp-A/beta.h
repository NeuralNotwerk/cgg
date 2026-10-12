#pragma once
#include <cstdio>
#include "processor.hpp"
struct Beta : Processor {
    void process() override;
    void unusedOp() override;
};
