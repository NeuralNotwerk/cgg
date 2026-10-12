import 'processor.dart';

class Zeta implements Processor {
  @override
  void process() {
    print('Zeta');
  }

  @override
  void unusedOp() {
    print('u');
  }
}
