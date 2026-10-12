class Other:
    def zork(self): pass
class A:
    def zork(self): pass
    def run(self, other):
        other.o.zork()
        self.o.zork()
        other.me().zork()
        A().zork()
    def me(self): return self
