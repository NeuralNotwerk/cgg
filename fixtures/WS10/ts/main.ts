import { Foo } from "./foo";
async function run(o: any, f: Foo, p: Promise<Foo>) {
  f.m();
  (f).m();
  (o as Foo).m();
  (<Foo>o).m();
  new Foo().m();
  (new Foo()).m();
  (await p).m();
  f!.m();
  f?.m();
}
