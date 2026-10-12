import pkg
from pkg import jobs
from pkg.jobs import Runner
from . import util


def go():
    pkg.helper()
    pkg.spawn()
    jobs.Runner().start()
    Runner().start()
    util.local_fn()
    pkg.missing()
