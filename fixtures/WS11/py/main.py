import typing
from typing import Optional, Union
from models import Foo, Bar


def a_control(x: Foo):
    x.run()


def b_optional(x: Optional[Foo]):
    x.run()


def c_pipe_none(x: Foo | None):
    x.run()


def d_forward(x: "Foo"):
    x.run()


def e_union_none(x: Union[Foo, None]):
    x.run()


def f_forward_optional(x: "Optional[Foo]"):
    x.run()


def g_multi(x: Foo | Bar):
    x.run()


def h_qualified(x: typing.Optional[Foo]):
    x.run()


def i_none_pipe(x: None | Foo):
    x.run()


def mk() -> Optional[Foo]:
    return None


def j_return():
    foo = mk()
    foo.run()


def k_local():
    y: Optional[Foo] = None
    y.run()


class K:
    z: Optional[Foo]

    def __init__(self, f: Optional[Foo]):
        self.f = f
        self.g: Optional[Foo] = None

    def m(self):
        self.f.run()

    def n(self):
        self.z.run()

    def o(self):
        self.g.run()


def p_untyped(x):
    x.run()


def q_local_plain():
    y: Foo = make()
    y.run()


def r_ret_forward() -> "Foo":
    return None


def s_use_forward():
    foo = r_ret_forward()
    foo.run()
