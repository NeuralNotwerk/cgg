#include <stddef.h>
#include "macros.h"

int cb(int x);
int handler(int x);
int step(void);
int fn(void);
int now(void);
int later(void);
int helper(void);

static void pass(int (*f)(void)) { (void)f; }

/* Parameter (a function pointer). */
static int with_param(int (*cb)(int)) { return cb(1); }

/* K&R parameter. */
static int with_kr(handler) int (*handler)(int); { return handler(2); }

/* Block-scope function-pointer variable. */
static int with_local(void)
{
    int (*step)(void) = 0;
    return step();
}

/* for init declaration. */
static int with_for(void)
{
    int t = 0;
    for (int (*fn)(void) = 0; t < 1; t++) {
        t += fn();
    }
    return t;
}

/* static local. */
static int with_static(void)
{
    static int (*handler)(int);
    return handler(3);
}

/* Value position through a parameter: no reference. */
static void pass_param(int (*helper)(void)) { pass(helper); }

/* A function-like macro expands whatever local shares its name. */
static int with_macro(int pick) { return pick(pick); }

/* Positive controls: an initializer sees the function; after the block
 * ends the function is visible again; a value named outside a block that
 * declares the name is the function. */
static int controls(void)
{
    int n = now();
    {
        int later = 1;
        (void)later;
    }
    pass(helper);
    return n + later();
}

int main(void)
{
    with_param(0);
    with_kr(0);
    with_local();
    with_for();
    with_static();
    pass_param(0);
    with_macro(0);
    return controls();
}
