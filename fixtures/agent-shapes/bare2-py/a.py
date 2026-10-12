class Repo:
    def zlist(self, d): return d
    async def azlist(self, d): return d
    def caller(self, data):
        zlist(data)
        azlist(data)
def top(data):
    zlist(data)
    azlist(data)
