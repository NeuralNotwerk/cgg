import 'processor.dart';

class Gamma implements Processor {
  @override
  void process() {
    print('Gamma');
  }

  @override
  void unusedOp() {
    print('u');
  }
}
