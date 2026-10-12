#pragma once
#include <cstdio>
#include "base.h"
class Alpha : public Base {
private:
    void process() override { std::puts("Alpha"); }
    void unusedOp() override { std::puts("u"); }
};
