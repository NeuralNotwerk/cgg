class Box {
  genM<T>(x: T): T { return x; }
  ctlM(x: number): number { return x; }
}
function genF<T>(x: T): T { return x; }
function ctlF(x: number): number { return x; }
export function useGen() {
  const b: Box = new Box();
  genF<number>(1);
  b.genM<string>("a");
}
export function useCtl() {
  const b: Box = new Box();
  ctlF(1);
  b.ctlM(2);
}
