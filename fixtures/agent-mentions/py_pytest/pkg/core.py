def add(a, b):
    return a + b


def _private_dead():
    return 0


class Thing:
    def __init__(self):
        self._v = 0

    @property
    def value(self):
        return self._v

    @value.setter
    def value(self, v):
        self._v = v
