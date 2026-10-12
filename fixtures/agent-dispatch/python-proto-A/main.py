from processor import Processor
from alpha import Alpha
from beta import Beta


def run(p: Processor) -> None:
    p.process()


def main() -> None:
    run(Alpha())
    run(Beta())


if __name__ == "__main__":
    main()
