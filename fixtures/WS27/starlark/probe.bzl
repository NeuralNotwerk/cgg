load("//x:y.bzl", "imp", alias = "name")
top = 1
def f(a, b = top, *args, c, d = 2, **kwargs):
    x = a()
    y, [z, w] = pair()
    n += 1
    for i, j in items:
        i()
    def inner(p):
        return p
    l = lambda q, r = 1: q(r)
    return [k() for k in ks if k] + {u: v for u, v in m.items()}
