def outer():
    def go():
        pass
    return go

def other():
    f = lambda go: go()
    return f
