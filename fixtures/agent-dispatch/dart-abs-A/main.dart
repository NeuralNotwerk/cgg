import 'processor.dart';
import 'alpha.dart';
import 'beta.dart';

void run(Processor p) {
  p.process();
}

void main() {
  run(Alpha());
  run(Beta());
}
