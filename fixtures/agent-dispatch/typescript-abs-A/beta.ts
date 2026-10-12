import { Processor } from "./processor";

export class Beta extends Processor {
  process(): void { console.log("Beta"); }
  unusedOp(): void { console.log("u"); }
}
