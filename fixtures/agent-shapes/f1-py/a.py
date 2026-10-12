class Repo:
    def quux(self): return 1
class Other:
    def quux(self): return 2
class Svc:
    def __init__(self, repo: Repo):
        self.repo = repo
        self.repo2 = Repo()
    def sync_m(self):
        self.repo.quux()
    async def async_m(self):
        self.repo.quux()
    def sync_m2(self):
        self.repo2.quux()
    async def async_m2(self):
        self.repo2.quux()
class Svc3:
    repo: Repo
    def sync_m3(self):
        self.repo.quux()
    async def async_m3(self):
        self.repo.quux()
