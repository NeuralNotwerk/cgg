#define G(n) int get_##n(void) { return 1; }
G(a)
G(b)
int real(void) { return get_a() + get_b(); }
static G(c)
