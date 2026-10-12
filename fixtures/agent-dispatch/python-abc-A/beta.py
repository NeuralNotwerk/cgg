from processor import Processor


class Beta(Processor):
    def process(self) -> None:
        print("Beta")

    def unused_op(self) -> None:
        print("u")
