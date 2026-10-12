import { Processor } from "./processor";

export class Zeta implements Processor {
  process(): void { console.log("Zeta"); }
  unusedOp(): void { console.log("u"); }
}
