def helper(x):
    return x

def run(xs):
    fn = lambda x: helper(x)
    return [fn(x) for x in xs]
