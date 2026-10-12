import Foo from './foo';
import { Runner } from './svc';
export function go() {
  Foo.create();
  new Runner();
  Runner.make();
}
