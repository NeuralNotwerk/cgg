import { Foo } from "./foo.js";
async function run(p) {
  const f = new Foo();
  f.m();
  (f).m();
  new Foo().m();
  (new Foo()).m();
  (await p).m();
}
