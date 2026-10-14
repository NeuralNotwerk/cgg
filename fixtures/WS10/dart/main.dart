import 'foo.dart';
Future<void> run(Object o, Foo f, Foo? n, Future<Foo> p) async {
  f.m();
  (f).m();
  (o as Foo).m();
  Foo().m();
  (Foo()).m();
  n?.m();
  (await p).m();
}
