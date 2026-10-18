class Comp {
  private onSave = (x: number): void => { this.persist(x); };
  public readonly onLoad = function (this: Comp) { this.persist(1); };
  count: number = 0;
  persist(x: number) { this.onSave(x); }
}
function boot(c: Comp) { c.onLoad(); }
