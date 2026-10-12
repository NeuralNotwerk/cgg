class Foo { m(x: number): number { return x; } }
function g<T>(x: T): T { return x; }
export function main() { const f = new Foo(); f.m(1); g<number>(2); zzz(3); }
