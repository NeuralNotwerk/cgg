class Foo { int zorkle() => 1; }
int ctlCall(Foo f) { return f.zorkle(); }
int nullSafe(Foo? f) { return f?.zorkle() ?? 0; }
int bang(Foo? f) { return f!.zorkle(); }
int cascade(Foo f) { f..zorkle(); return 0; }
