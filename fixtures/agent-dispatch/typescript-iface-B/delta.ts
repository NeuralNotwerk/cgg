import { Processor } from "./processor";

export class Delta implements Processor {
  process(): void { console.log("Delta"); }
  unusedOp(): void { console.log("u"); }
}
