import { Processor } from "./processor";

export class Gamma extends Processor {
  process(): void { console.log("Gamma"); }
  unusedOp(): void { console.log("u"); }
}
