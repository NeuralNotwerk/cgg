#pragma once
#include <cstdio>
#include "processor.h"
struct Beta : Processor {
    void process() override;
    void unusedOp() override;
};
