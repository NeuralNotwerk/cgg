import { Processor } from "./processor";
import { Alpha } from "./alpha";
import { Beta } from "./beta";

class Holder {
  constructor(private field: Processor) {}
  runField(): void { this.field.process(); }
}

export function main(): void {
  new Holder(new Alpha()).runField();
  new Holder(new Beta()).runField();
}

main();
