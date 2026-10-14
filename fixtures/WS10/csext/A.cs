namespace N {
  class Foo { public void M() {} }
  class Bar { public void Ext() {} }
  static class FooExt { public static void Ext(this Foo f) {} }
  class Main {
    void Run(object o, Foo f) {
      f.Ext();
      ((Foo)o).Ext();
      new Foo().Ext();
      (o as Foo)?.Ext();
    }
  }
}
