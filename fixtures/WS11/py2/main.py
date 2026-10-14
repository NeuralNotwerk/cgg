from models import Foo, Bar


def a_default(x: Foo = None):
    x.run()


class K:
    def __init__(self, f: Foo):
        self.f = f

    def m(self):
        self.f.run()
