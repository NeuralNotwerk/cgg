import signal, atexit
def on_sig(n, f): pass
def main():
    signal.signal(signal.SIGINT, on_sig)
