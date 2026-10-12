class Box {
  T genM<T>(T x) => x;
  int ctlM(int x) => x;
}
T genF<T>(T x) => x;
int ctlF(int x) => x;
void useGen() {
  var b = Box();
  genF<int>(1);
  b.genM<String>("a");
}
void useCtl() {
  var b = Box();
  ctlF(1);
  b.ctlM(2);
}
