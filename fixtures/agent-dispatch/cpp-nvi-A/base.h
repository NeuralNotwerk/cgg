#pragma once
class Base {
public:
    virtual ~Base() = default;
    void run() { process(); }
private:
    virtual void process() = 0;
    virtual void unusedOp() = 0;
};
