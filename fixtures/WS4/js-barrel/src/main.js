import * as lib from './lib';
export function go() {
  lib.f();
  lib.h();
  lib.u.helper();
  lib.Runner.make();
  new lib.Runner().start();
  lib.missing();
}
