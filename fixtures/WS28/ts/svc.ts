export class Svc {
  run(): void;
  run(a: number): void;
  run(a?: number, b?: number): void {}
  find(id: string, opts?: object): void {}
  find2(id: string): void {}
}
export function make(a: number): Svc;
export function make(a: number, b: string): Svc;
export function make(a: number, b?: string): Svc { return new Svc(); }
export class Other {
  find(a: string, b: string, c: string): void {}
}
