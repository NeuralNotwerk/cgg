export abstract class Base {
  run(): void {
    this.process();
  }
  protected abstract process(): void;
  protected abstract unusedOp(): void;
}
