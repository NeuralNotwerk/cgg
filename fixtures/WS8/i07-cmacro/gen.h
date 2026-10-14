#define DEF_GETTER(n) int get_##n(void) { return n; }
#define LIST(X) X(alpha) X(beta)
#define DECL_HANDLER(name) void handle_##name(int v) { (void)v; }
int helper_real(void);
