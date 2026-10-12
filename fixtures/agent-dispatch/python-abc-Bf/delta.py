from processor import Processor


class Delta(Processor):
    def process(self) -> None:
        print("Delta")

    def unused_op(self) -> None:
        print("u")
