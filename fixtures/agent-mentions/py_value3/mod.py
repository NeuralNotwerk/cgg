import functools
from unittest import mock


def _decorator(fn):
    return fn


@_decorator
def _decorated():
    return 1


def _default_cb():
    return 2


def _with_default(cb=_default_cb):
    return cb()


def _yielded():
    return 3


def _gen():
    yield _yielded


def _tuple_a():
    return 4


def _tuple_b():
    return 5


def _pair():
    return (_tuple_a, _tuple_b)


def _attr_assigned():
    return 6


def _comprehension_target():
    return 7


def _setattr_target():
    return 8


def _is_compared():
    return 9


def _lambda_called():
    return 10


def _patched_by_string():
    return 11


def _bool_op_target():
    return 12


class _Impl:
    def _call_impl(self):
        return 13

    __call__ = _call_impl


class _Obj:
    pass


def main():
    o = _Obj()
    o.cb = _attr_assigned
    d = {k: _comprehension_target for k in "ab"}
    setattr(o, "x", _setattr_target)
    same = o.cb is _is_compared
    f = lambda: _lambda_called()
    g = None or _bool_op_target
    with mock.patch("mod._patched_by_string"):
        pass
    return _decorated, _with_default(), list(_gen()), _pair(), d, same, f, g, _Impl()


if __name__ == "__main__":
    main()
