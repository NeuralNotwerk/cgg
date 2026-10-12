from typing import Optional, List
class Foo:
    def zorkle(self): return 1
class Bar:
    def zorkle(self): return 2
def ctl_typed(x: Foo): x.zorkle()
def ctl_untyped(x): x.zorkle()
def opt(x: Optional[Foo]): x.zorkle()
def union_none(x: Foo | None): x.zorkle()
def str_ann(x: "Foo"): x.zorkle()
def list_iter(xs: list[Foo]):
    for x in xs:
        x.zorkle()
def typing_list_iter(xs: List[Foo]):
    for x in xs:
        x.zorkle()
def local_ann():
    x: Optional[Foo] = None
    x.zorkle()
def opt_kw(x: Optional[Foo] = None):
    x.zorkle()
