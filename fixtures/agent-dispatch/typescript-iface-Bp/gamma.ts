import { Processor } from "./processor";

export class Gamma implements Processor {
  process(): void { console.log("Gamma"); }
  unusedOp(): void { console.log("u"); }
}
