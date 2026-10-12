#pragma once
#include <cstdio>
#include "base.h"
class Beta : public Base {
private:
    void process() override { std::puts("Beta"); }
    void unusedOp() override { std::puts("u"); }
};
