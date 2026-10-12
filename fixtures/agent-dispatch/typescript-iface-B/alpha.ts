import { Processor } from "./processor";

export class Alpha implements Processor {
  process(): void { console.log("Alpha"); }
  unusedOp(): void { console.log("u"); }
}
