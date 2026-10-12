import atexit
import threading


def _map_target(x):
    return x + 1


def _sort_key(x):
    return -x


def _thread_target():
    print("t")


def _atexit_target():
    print("bye")


def _dispatch_a():
    return "a"


def _getattr_target():
    return "g"


def _dead_control():
    return "nobody"


def _called_control():
    return "live"


class _Worker:
    def _bound_method_target(self):
        return 1

    def _getattr_method(self):
        return 2

    def run(self):
        return list(map(self._bound_method_target, [1]))


TABLE = {"a": _dispatch_a}


def main():
    xs = [3, 1, 2]
    ys = list(map(_map_target, xs))
    zs = sorted(xs, key=_sort_key)
    t = threading.Thread(target=_thread_target)
    t.start()
    atexit.register(_atexit_target)
    w = _Worker()
    w.run()
    getattr(w, "_getattr_method")()
    TABLE["a"]()
    _called_control()
    return ys, zs


if __name__ == "__main__":
    main()
