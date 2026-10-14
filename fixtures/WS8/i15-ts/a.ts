abstract class Base {
  abstract run(): void;
  protected helper(): number { return 1; }
  private secret(): number { return this.helper(); }
  go(): number { return this.secret(); }
}
class Impl extends Base {
  run(): void { this.helper(); }
}
