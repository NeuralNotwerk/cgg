#include <stdlib.h>

struct ops { int (*open)(void); int (*close)(void); };

static int my_open(void) { return 0; }
static int my_close(void) { return 0; }
static int in_array(void) { return 1; }
static int assigned(void) { return 2; }
static int cmp(const void *a, const void *b) { return 0; }
static int returned_fn(void) { return 3; }
static int addr_of(void) { return 4; }
static int casted(void) { return 5; }
static int orphan(void) { return 6; }     /* control: dead */
static int shadowed(void) { return 7; }   /* near-miss: only a parameter of this name is passed */
static int in_string(void) { return 8; }  /* near-miss: only a string */

static const struct ops table = { .open = my_open, my_close };
static int (*arr[])(void) = { in_array };
static int limit = 3;

static int (*pick(void))(void) { return returned_fn; }
static void use(int (*f)(void)) { (void)f; }
static void use_int(int x) { (void)x; }
static void take(int (*shadowed)(void)) { use(shadowed); }

int main(void) {
    struct ops o;
    int xs[2] = {2, 1};
    o.open = assigned;
    qsort(xs, 2, sizeof(int), cmp);
    use(&addr_of);
    void *p = (void *)casted;
    puts("in_string");
    use_int(limit);
    (void)table; (void)arr; (void)p;
    pick();
    take(0);
    return o.open();
}
