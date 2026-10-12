import { Processor } from "./processor";

export class Zeta extends Processor {
  process(): void { console.log("Zeta"); }
  unusedOp(): void { console.log("u"); }
}
