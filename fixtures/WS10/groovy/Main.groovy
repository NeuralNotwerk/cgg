class Main {
  def run(Object o, Foo f) {
    f.m()
    (f).m()
    ((Foo) o).m()
    (o as Foo).m()
    new Foo().m()
    f?.m()
  }
}
