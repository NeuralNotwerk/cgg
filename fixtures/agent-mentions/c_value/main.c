#include <stdlib.h>
#include <pthread.h>

struct file_ops { int (*open)(void); void (*close)(void); };

static int cmp_fn(const void *a, const void *b) { return 0; }
static int my_open(void) { return 0; }
static void my_close(void) {}
static void *thread_fn(void *arg) { return arg; }
static void atexit_fn(void) {}
static int table_a(void) { return 1; }
static void dead_control(void) {}
static void called_control(void) {}

static const struct file_ops ops = { .open = my_open, .close = my_close };
static int (*table[])(void) = { table_a };

int main(void) {
    int a[3] = {3, 1, 2};
    pthread_t t;
    qsort(a, 3, sizeof(int), cmp_fn);
    pthread_create(&t, NULL, thread_fn, NULL);
    atexit(atexit_fn);
    called_control();
    return ops.open() + table[0]();
}
