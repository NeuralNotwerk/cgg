from processor import Processor


class Alpha(Processor):
    def process(self) -> None:
        print("Alpha")

    def unused_op(self) -> None:
        print("u")
