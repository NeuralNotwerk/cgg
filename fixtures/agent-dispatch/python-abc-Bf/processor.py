from abc import ABC, abstractmethod


class Processor(ABC):
    @abstractmethod
    def process(self) -> None: ...

    @abstractmethod
    def unused_op(self) -> None: ...
