#include "gen.h"
static int x = 1;
DEF_GETTER(x)
FIELDS(DECL_HANDLER)
int helper_real(void) { return 2; }
