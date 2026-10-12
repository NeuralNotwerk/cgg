function mapTarget(x: number): number { return x + 1; }
function clickHandler(e: Event): void { console.log(e); }
function timeoutTarget(): void { console.log("t"); }
function exportedHandler(): number { return 1; }
function deadControl(): number { return 0; }
function calledControl(): number { return 5; }

export class Widget {
  private privDead(): number { return 0; }
  private privMapTarget(x: number): number { return x; }
  private privClick(): void { }
  private privTimeout(): void { }
  private privAssigned(): number { return 1; }
  private privCalled(): number { return 2; }
  private privReturned(): number { return 3; }
  public render(): number[] {
    const f = this.privAssigned;
    setTimeout(this.privTimeout, 1);
    document.body.addEventListener("click", this.privClick);
    this.privCalled();
    return [1].map(this.privMapTarget);
  }
  public getFn() { return this.privReturned; }
  public lambdaUse() { return [1].map(x => this.privLambda(x)); }
  private privLambda(x: number) { return x; }
}

function main() {
  const xs = [1, 2, 3].map(mapTarget);
  document.body.addEventListener("click", clickHandler);
  setTimeout(timeoutTarget, 10);
  calledControl();
  return xs;
}
main();
export default { handler: exportedHandler };
