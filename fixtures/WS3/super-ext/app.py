import torch.nn as nn
class Other:
    def forward(self, x): return x
class Third:
    def forward(self, x): return x
class Net(nn.Module):
    def forward(self, x):
        return super().forward(x)
class Base:
    def go(self): pass
class Kid(Base):
    def go(self):
        super().go()
