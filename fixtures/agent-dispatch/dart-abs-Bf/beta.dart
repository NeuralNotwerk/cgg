import 'processor.dart';

class Beta implements Processor {
  @override
  void process() {
    print('Beta');
  }

  @override
  void unusedOp() {
    print('u');
  }
}
