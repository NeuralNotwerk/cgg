#include "local.h"

static int start(void *p) { (void)p; return 0; }
static void handler(int sig) { (void)sig; }
static int width(void) { return 0; }

int spawn(int (*fn)(void *), void *arg);
int on_signal(void (*fn)(int));

int main(void)
{
    spawn(start, 0);
    on_signal(handler);
    struct shape s;
    s.width = 1;
    s.height = 2;
    spawn((int (*)(void *))width, 0);
    return spawn((int (*)(void *))height, 0);
}
