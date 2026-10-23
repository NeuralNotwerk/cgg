import { Svc, make } from "./svc";
export function go(s: Svc, o: any) {
  s.run();
  s.run(1, 2);
  make(1);
  o.find("x");
}
