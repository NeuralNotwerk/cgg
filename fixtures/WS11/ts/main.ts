import { Foo, Bar } from "./models";

export function aControl(x: Foo) {
  x.run();
}
export function bUndef(x: Foo | undefined) {
  x.run();
}
export function cNull(x: Foo | null) {
  x.run();
}
export function dOptParam(x?: Foo) {
  x.run();
}
export function eMulti(x: Foo | Bar) {
  x.run();
}
export function fNullFirst(x: null | Foo) {
  x.run();
}
export function gUntyped(x) {
  x.run();
}
function mk(): Foo | undefined {
  return undefined;
}
export function hRet() {
  const foo = mk();
  foo.run();
}
export function iLocal() {
  let y: Foo | null = null;
  y.run();
}
export class K {
  private f: Foo | null = null;
  constructor(private g: Foo | undefined) {}
  m() {
    this.f.run();
  }
  n() {
    this.g.run();
  }
}
function mk2(): Foo {
  return new Foo();
}
export function jRetPlain() {
  const foo = mk2();
  foo.run();
}
export function kLocalPlain() {
  let y: Foo = new Foo();
  y.run();
}
export class K2 {
  private f: Foo;
  m() {
    this.f.run();
  }
}
