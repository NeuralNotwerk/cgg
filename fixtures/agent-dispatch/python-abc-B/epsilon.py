from processor import Processor


class Epsilon(Processor):
    def process(self) -> None:
        print("Epsilon")

    def unused_op(self) -> None:
        print("u")
