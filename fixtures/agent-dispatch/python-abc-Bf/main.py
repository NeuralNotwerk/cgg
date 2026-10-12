from processor import Processor
from alpha import Alpha
from beta import Beta
from gamma import Gamma
from delta import Delta
from epsilon import Epsilon
from zeta import Zeta


class Holder:
    def __init__(self, field: Processor) -> None:
        self.field = field

    def run_field(self) -> None:
        self.field.process()


def main() -> None:
    Holder(Alpha()).run_field()
    Holder(Beta()).run_field()


if __name__ == "__main__":
    main()
