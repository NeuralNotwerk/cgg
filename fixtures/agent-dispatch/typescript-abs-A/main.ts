import { Processor } from "./processor";
import { Alpha } from "./alpha";
import { Beta } from "./beta";

function run(p: Processor): void {
  p.process();
}

export function main(): void {
  run(new Alpha());
  run(new Beta());
}

main();
