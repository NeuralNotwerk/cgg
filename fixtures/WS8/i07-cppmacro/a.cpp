#define FIELDS(X) X(alpha)
#define DECL_HANDLER(n) void handle_##n(int v) {}
static int x = 1;
FIELDS(DECL_HANDLER)
int helper_real(void) { return 2; }
int user() { return helper_real(); }
