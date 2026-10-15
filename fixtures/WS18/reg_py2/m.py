import signal
import atexit
def on_sig(n, f): pass
def on_exit(): pass
def main():
    signal.signal(signal.SIGINT, on_sig)
    atexit.register(on_exit)
