using System.Threading.Tasks;
class Main {
  async Task Run(object o, Foo f, Task<Foo> t) {
    f.M();
    (f).M();
    ((Foo)o).M();
    (o as Foo).M();
    new Foo().M();
    new Foo { X = 1 }.M();
    (await t).M();
    f?.M();
    (o as Foo)?.M();
  }
}
