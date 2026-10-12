import { Processor } from "./processor";

export class Beta implements Processor {
  process(): void { console.log("Beta"); }
  unusedOp(): void { console.log("u"); }
}
