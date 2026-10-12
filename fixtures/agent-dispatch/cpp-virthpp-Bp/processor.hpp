#pragma once
struct Processor {
    virtual ~Processor() = default;
    virtual void process() = 0;
    virtual void unusedOp() = 0;
};
