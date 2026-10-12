import { Processor } from "./processor";
import { Alpha } from "./alpha";
import { Beta } from "./beta";
import { Gamma } from "./gamma";
import { Delta } from "./delta";
import { Epsilon } from "./epsilon";
import { Zeta } from "./zeta";

function run(p: any): void {
  p.process();
}

export function main(): void {
  run(new Alpha());
  run(new Beta());
  run(new Gamma());
  run(new Delta());
  run(new Epsilon());
  run(new Zeta());
}

main();
