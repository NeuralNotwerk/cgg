#define G(n) int get_##n() { return 1; }
G(a)
G(b)
int real() { return get_a() + get_b(); }
