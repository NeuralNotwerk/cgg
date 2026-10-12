import { Base } from "./base";
export class Beta extends Base {
  protected process(): void { console.log("Beta"); }
  protected unusedOp(): void { console.log("u"); }
}
