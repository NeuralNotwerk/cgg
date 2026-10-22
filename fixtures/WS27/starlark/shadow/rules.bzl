# Functions whose names the parameters and locals below reuse.
def cb(x):
    return x
def handler(x):
    return x
def item(x):
    return x
def step(x):
    return x
def helper(x):
    return x
def now():
    return 0

# A parameter, a `lambda` parameter, a local, a `for` target.
def with_param(cb):
    return cb(1)
def with_lambda(xs):
    return [f(2) for f in xs] + [(lambda handler: handler(3))(cb)]
def with_local():
    item = now()
    return item(4)
def with_for(fs):
    for step in fs:
        step(5)

# A nested def keeps its edge; another function's call reaches the
# module's `helper`.
def with_nested():
    def helper(x):
        return x + 1
    return helper(6)
def with_module():
    return helper(7)

# A default sees the module's function; a keyword argument binds nothing.
def with_default(v = now()):
    return cb(v, handler = handler)
