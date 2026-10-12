import functools
import signal
from pkg.helpers import _xfile_value, _xfile_called
from pkg import helpers


def _in_func_dict():
    return "d"


def _list_member():
    return "l"


def _assigned():
    return "a"


def _partial_target(a, b):
    return a + b


def _signal_handler(signum, frame):
    pass


def _class_attr_cb():
    return "c"


def _kw_callback():
    return "k"


def _return_value():
    return "r"


class _Holder:
    callback = _class_attr_cb


def _make():
    return _return_value


def _register(fn, **kw):
    return fn


def run():
    table = {"x": _in_func_dict}
    fns = [_list_member]
    h = _assigned
    p = functools.partial(_partial_target, 1)
    signal.signal(signal.SIGINT, _signal_handler)
    _register(_xfile_value)
    _xfile_called()
    getattr(helpers, "_xfile_getattr")()
    _register(None, on_done=_kw_callback)
    f = _make()
    return table, fns, h, p, _Holder, f


if __name__ == "__main__":
    run()
