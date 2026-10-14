from foo import Foo
async def run(f: Foo, p):
    f.m()
    (f).m()
    Foo().m()
    (Foo()).m()
    (await p).m()
