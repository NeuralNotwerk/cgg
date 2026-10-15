import atexit, signal
from pyomo.environ import Var
def _s_init(m, t): return 0
def on_exit(): pass
def on_sig(n, f): pass
def dead(): pass
def build():
    Var(initialize=_s_init)
atexit.register(on_exit)
signal.signal(signal.SIGINT, on_sig)
build()
