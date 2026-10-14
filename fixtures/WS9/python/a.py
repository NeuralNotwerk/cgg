def f[T](x: T) -> T:
    return x
class Box[T]:
    def m(self): pass
def caller():
    f[int](1)
    Box[int]().m()
    f(2)
