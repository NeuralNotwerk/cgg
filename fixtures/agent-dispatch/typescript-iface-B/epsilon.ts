import { Processor } from "./processor";

export class Epsilon implements Processor {
  process(): void { console.log("Epsilon"); }
  unusedOp(): void { console.log("u"); }
}
