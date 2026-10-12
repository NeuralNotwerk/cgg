import { Processor } from "./processor";

export class Epsilon extends Processor {
  process(): void { console.log("Epsilon"); }
  unusedOp(): void { console.log("u"); }
}
