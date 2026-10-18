#include <thread>
#include <chrono>
void f() {
    std::this_thread::sleep_for(std::chrono::seconds(1));
    std::sort(nullptr, nullptr);
}
