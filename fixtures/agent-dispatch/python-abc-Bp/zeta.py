from processor import Processor


class Zeta(Processor):
    def process(self) -> None:
        print("Zeta")

    def unused_op(self) -> None:
        print("u")
