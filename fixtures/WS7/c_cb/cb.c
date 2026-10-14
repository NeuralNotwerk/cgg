#include <signal.h>
#include <stdlib.h>
#include <pthread.h>

static void on_signal(int s) { (void)s; }
static void on_exit_hook(void) {}
static void *worker(void *a) { return a; }
static int cmp(const void *a, const void *b) { return 0; }
static void once_init(void) {}
/* near miss */
static void never_used(void) {}

int main(void) {
    pthread_t t;
    static pthread_once_t o = PTHREAD_ONCE_INIT;
    int xs[3] = {3, 1, 2};
    signal(SIGINT, on_signal);
    atexit(on_exit_hook);
    pthread_create(&t, 0, worker, 0);
    qsort(xs, 3, sizeof(int), cmp);
    pthread_once(&o, once_init);
    return 0;
}
