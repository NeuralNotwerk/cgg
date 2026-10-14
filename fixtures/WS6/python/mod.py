import functools


def _ret_target():
    pass


def _yield_target():
    pass


def _deco(f):
    return f


def _default_target():
    pass


def _comp_target():
    pass


def _is_target():
    pass


def _chain_target():
    pass


def _list_comp_target():
    pass


def _never_referenced():
    pass


def get_ret():
    return _ret_target


def gen():
    yield _yield_target


@_deco
def decorated():
    pass


def with_default(cb=_default_target):
    cb()


def comp(keys):
    d = {k: _comp_target for k in keys}
    l = [_list_comp_target for _ in keys]
    return d, l


def check(x):
    return x is _is_target


def chained():
    a = b = _chain_target
    return a, b


def main():
    get_ret()
    list(gen())
    decorated()
    with_default()
    comp([1])
    check(1)
    chained()


if __name__ == "__main__":
    main()
