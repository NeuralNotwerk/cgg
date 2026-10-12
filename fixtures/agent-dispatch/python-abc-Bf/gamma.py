from processor import Processor


class Gamma(Processor):
    def process(self) -> None:
        print("Gamma")

    def unused_op(self) -> None:
        print("u")
