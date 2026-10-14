#include "gen.h"
int helper_real(void) { return 2; }
static int x = 1;
DEF_GETTER(x)
LIST(DECL_HANDLER)
