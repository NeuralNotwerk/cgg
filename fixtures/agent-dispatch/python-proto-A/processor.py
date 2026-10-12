from typing import Protocol


class Processor(Protocol):
    def process(self) -> None: ...

    def unused_op(self) -> None: ...
