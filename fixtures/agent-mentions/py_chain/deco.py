import functools


def public_deco(fn):
    @functools.wraps(fn)
    def wrapper(*a, **k):
        return fn(*a, **k)
    return wrapper


def other_deco(fn):
    def _inner(*a):
        return fn(*a)
    return _inner


def _private_deco(fn):
    return fn


@public_deco
@other_deco
@_private_deco
def target():
    return 1


class _Cls:
    def _fail(self, *a):
        raise RuntimeError

    __setitem__ = __delitem__ = _fail


    def _alias_target(self):
        return 1

    alias = _alias_target


if __name__ == "__main__":
    target()
    _Cls()
