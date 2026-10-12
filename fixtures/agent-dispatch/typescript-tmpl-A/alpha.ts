import { Base } from "./base";
export class Alpha extends Base {
  protected process(): void { console.log("Alpha"); }
  protected unusedOp(): void { console.log("u"); }
}
