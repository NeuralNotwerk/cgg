#include <csignal>
#include <cstdlib>
#include <thread>
#include <algorithm>

static void on_signal(int) {}
static void on_exit_hook() {}
static void run() {}
static bool less_than(int a, int b) { return a < b; }
// near miss
static void never_used() {}

int main() {
    std::signal(SIGINT, on_signal);
    std::atexit(on_exit_hook);
    std::thread t(run);
    int xs[3] = {3, 1, 2};
    std::sort(xs, xs + 3, less_than);
    t.join();
    return 0;
}
