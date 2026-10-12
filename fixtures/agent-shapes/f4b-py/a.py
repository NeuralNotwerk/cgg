class Repo:
    def quux(self): return 1
class Other:
    def quux(self): return 2
class Inner:
    def __init__(self):
        self.repo = Repo()
class A:
    def __init__(self):
        self.repo = Repo()
        self.inner = Inner()
    def ctl_field(self):
        self.repo.quux()
    def multi_recv(self):
        (self
            .repo
            .quux())
    def ctl_two(self):
        self.inner.repo.quux()
    def multi_two(self):
        (self.inner
            .repo
            .quux())
