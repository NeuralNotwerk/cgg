class Foo { zorkle(): number { return 1; } }
class Bar { zorkle(): number { return 2; } }
export function ctlTyped(x: Foo) { x.zorkle(); }
export function ctlUntyped(x: any) { x.zorkle(); }
export function unionUndef(x: Foo | undefined) { x?.zorkle(); }
export function unionNull(x: Foo | null) { x!.zorkle(); }
export function optParam(x?: Foo) { x?.zorkle(); }
export function optChain(x: Foo) { x?.zorkle(); }
export function nonNull(x: Foo) { x!.zorkle(); }
export async function promise(p: Promise<Foo>) { (await p).zorkle(); }
export async function awaitLocal(p: Promise<Foo>) { const x = await p; x.zorkle(); }
export function arr(xs: Foo[]) { for (const x of xs) { x.zorkle(); } }
export function arrGeneric(xs: Array<Foo>) { xs[0].zorkle(); }
