import 'processor.dart';

class Epsilon implements Processor {
  @override
  void process() {
    print('Epsilon');
  }

  @override
  void unusedOp() {
    print('u');
  }
}
