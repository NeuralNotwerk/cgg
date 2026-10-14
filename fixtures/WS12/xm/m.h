#define LOCK(n) static struct lock n = { 0, { 1 } }
#define LIST(X) X(a) X(b)
#define DECL(n) void h_##n(void) { }
