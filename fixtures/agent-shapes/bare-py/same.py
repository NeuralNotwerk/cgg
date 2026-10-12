class Repo2:
    def ylist(self, d): return d
    async def aylist(self, d): return d
def use_same(data):
    ylist(data)
    aylist(data)
