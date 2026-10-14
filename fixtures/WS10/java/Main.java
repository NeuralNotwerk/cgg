class Main {
  void run(Object o, Foo f) {
    f.m();
    (f).m();
    ((Foo) o).m();
    new Foo().m();
    (new Foo()).m();
  }
}
