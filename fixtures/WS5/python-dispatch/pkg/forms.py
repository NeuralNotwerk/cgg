from django.forms import ModelForm
from pkg.base import Base


class MyForm(ModelForm):
    def _save_m2m(self):
        """Overrides a hook of an out-of-graph base."""
        return 1

    def __mangled(self):
        return 2


class Plain:
    def _helper(self):
        return 3


class Child(Base):
    def _step(self):
        return 4


def run(obj):
    obj._process()
