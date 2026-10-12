class Foo:
    def zorkle(self): return 1
class Bar:
    def zorkle(self): return 2
class Repo:
    def quux(self): return 1
class Other:
    def quux(self): return 2

class A:
    def __init__(self):
        self.repo = Repo()
    async def get(self) -> Foo:
        return Foo()
    def ctl_local(self):
        f = Foo()
        f.zorkle()
    def multi_local(self):
        f = Foo()
        (f
            .zorkle())
    def paren_local(self):
        f = Foo()
        (f).zorkle()
    def ctor_recv(self):
        Foo().zorkle()
    def ctl_field(self):
        self.repo.quux()
    def multi_field(self):
        (self.repo
            .quux())
    async def awaited(self):
        (await self.get()).zorkle()
    async def awaited_ctl(self):
        x = await self.get()
        x.zorkle()
