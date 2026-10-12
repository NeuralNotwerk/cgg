def _chain_fn_level():
    return 1


def _single_cls():
    return 2


def _chain_cls():
    return 3


class K:
    single = _single_cls
    a = b = _chain_cls


def main():
    x = y = _chain_fn_level
    return x, y, K


if __name__ == "__main__":
    main()
