import 'package:test/test.dart';
import 'fixtures.dart' as fixtures;

void main() {
  // `expect` comes from package:test (not analyzed): no edge.
  expect(1, 1);
  // Near miss: the imported file's own function binds.
  load();
}
