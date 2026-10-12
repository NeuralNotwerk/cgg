import pkg
import pkg.sub.jobs as jm
from . import helpers

def go():
    pkg.sub.jobs.spawn_job(1)
    pkg.sub.jobs.Runner().start_runner()
    jm.spawn_job(2)
    helpers.helper_fn()
