import 'processor.dart';

class Alpha implements Processor {
  @override
  void process() {
    print('Alpha');
  }

  @override
  void unusedOp() {
    print('u');
  }
}
