export interface Shape {
  area(): number;
  name?(): string;
}

export abstract class Base {
  abstract render(): void;
  draw(): void { this.render(); }
}

export class Square extends Base implements Shape {
  area(): number { return 1; }
  render(): void {}
}

export class Circle implements Shape {
  area(): number { return 3; }
}

export class Other {
  area(): number { return 99; }
}

export function total(s: Shape): number {
  return s.area();
}

export function paint(b: Base): void {
  b.render();
}
