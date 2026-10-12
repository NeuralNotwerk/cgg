import 'processor.dart';

class Delta implements Processor {
  @override
  void process() {
    print('Delta');
  }

  @override
  void unusedOp() {
    print('u');
  }
}
