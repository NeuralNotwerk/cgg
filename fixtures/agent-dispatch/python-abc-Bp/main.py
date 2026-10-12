from processor import Processor
from alpha import Alpha
from beta import Beta
from gamma import Gamma
from delta import Delta
from epsilon import Epsilon
from zeta import Zeta


def run(p: Processor) -> None:
    p.process()


def main() -> None:
    run(Alpha())
    run(Beta())
    run(Gamma())
    run(Delta())
    run(Epsilon())
    run(Zeta())


if __name__ == "__main__":
    main()
