from processor import Processor
from alpha import Alpha
from beta import Beta


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
