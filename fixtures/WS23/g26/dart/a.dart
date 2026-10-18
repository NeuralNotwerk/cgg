class Tool { void work() {} }
class Other { void work() {} }
Tool make() => Tool();
Other other() { return Other(); }
void use() {
  var t = make();
  t.work();
  make().work();
  final o = other();
  o.work();
}
void a() { var x = make(); x.work(); }
void b() { var x = other(); x.work(); }
void shadow() {
  var x = make();
  x.work();
  {
    var x = other();
    x.work();
  }
  [1].forEach((x) { x.work(); });
  x.work();
}
