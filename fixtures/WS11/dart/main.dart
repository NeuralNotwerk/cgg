import 'models.dart';

void aControl(Foo x) {
  x.run();
}

void bNullable(Foo? x) {
  x?.run();
}

void cBang(Foo? x) {
  x!.run();
}

void dNamed({Foo? x, required Foo y}) {
  x?.run();
  y.run();
}

void eClosure(List<Foo> xs) {
  xs.forEach((x) => x.run());
}

void fTypedClosure(List<Foo> xs) {
  xs.forEach((Foo x) => x.run());
}

void gLocal() {
  Foo? y = null;
  y?.run();
}

void hLocalPlain() {
  Foo y = Foo();
  y.run();
}

Foo? mk() => null;

void iRet() {
  var foo = mk();
  foo?.run();
}

void jUntyped(x) {
  x.run();
}

class K {
  Foo? f;
  Foo g = Foo();
  void m() {
    f?.run();
  }
  void n() {
    g.run();
  }
}

void each(void Function(Foo) cb) {}

void kCallbackTyped() {
  each((x) => x.run());
}

void lBlockClosure(List<Foo> xs) {
  xs.forEach((x) {
    x.run();
  });
}

void mTypedBlock(List<Foo> xs) {
  xs.forEach((Foo x) {
    x.run();
  });
}

void nCallbackBlock() {
  each((x) {
    x.run();
  });
}
