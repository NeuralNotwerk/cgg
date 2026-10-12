import { Processor } from "./processor";

export class Delta extends Processor {
  process(): void { console.log("Delta"); }
  unusedOp(): void { console.log("u"); }
}
