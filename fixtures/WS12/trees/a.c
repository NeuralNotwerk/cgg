static int x = 1;
DEF_GETTER(x)
FIELDS(DECL_HANDLER)
int helper_real(void) { return 2; }
DEF_GETTER(y);
EXPORT_SYMBOL(helper_real);
DEF_GETTER(z)
