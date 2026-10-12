import 'processor.dart';
import 'alpha.dart';
import 'beta.dart';
import 'gamma.dart';
import 'delta.dart';
import 'epsilon.dart';
import 'zeta.dart';

void run(Processor p) {
  p.process();
}

void main() {
  run(Alpha());
  run(Beta());
  run(Gamma());
  run(Delta());
  run(Epsilon());
  run(Zeta());
}
