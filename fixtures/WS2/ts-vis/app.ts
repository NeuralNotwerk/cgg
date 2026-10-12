export class Service {
  constructor(private readonly name: string) {}

  public run(): void {
    this.helper();
  }

  private helper(): void {}

  private unusedPrivate(): void {}

  protected unusedProtected(): void {}

  public unusedPublic(): void {}

  unusedNoModifier(): void {}

  #hardPrivateUnused(): void {}

  @Decorated("x")
  private decoratedPrivateUnused(): void {}

  static create(): Service { return new Service("x"); }

  private static staticPrivateUnused(): void {}

  get value(): string { return this.name; }

  private get hiddenValue(): string { return this.name; }
}

abstract class Base {
  protected abstract hook(): void;
  public template(): void { this.hook(); }
}

class Singleton {
  private constructor() {}
}

function freeFn(): void {}
