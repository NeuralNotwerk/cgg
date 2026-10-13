class Base:
    def _step(self):
        return 0

    def go(self):
        return self._step()


class Worker:
    def _process(self):
        return 5

    def _unrelated(self):
        return 6
