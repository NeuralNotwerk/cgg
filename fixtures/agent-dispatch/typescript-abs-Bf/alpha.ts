import { Processor } from "./processor";

export class Alpha extends Processor {
  process(): void { console.log("Alpha"); }
  unusedOp(): void { console.log("u"); }
}
