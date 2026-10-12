import 'processor.dart';
import 'alpha.dart';
import 'beta.dart';

class Holder {
  final Processor field;
  Holder(this.field);
  void runField() {
    field.process();
  }
}

void main() {
  Holder(Alpha()).runField();
  Holder(Beta()).runField();
}
