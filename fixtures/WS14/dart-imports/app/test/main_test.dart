import 'package:pkg_a/pkg_a.dart';
import 'package:test/test.dart';
import '../lib/util.dart' as u;
import 'package:app/util.dart' show utilFn;

void main() {
  helperOne();
  otherFn();
  helperTwo();
  expect(1);
  u.utilFn();
  utilFn();
  final cb = makeCb();
  cb();
}
int makeCb() => 0;
