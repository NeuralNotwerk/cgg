#include "gen.h"
static int y = 1;
DEF_GETTER(y)
int after_getter(void) { return helper_real(); }
LIST(DECL_HANDLER)
static int after_list(int z) { return z; }
int tail(void) { return after_list(1) + after_getter(); }
