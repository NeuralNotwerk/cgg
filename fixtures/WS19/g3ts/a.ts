function outer(a: number): void {
  function go(): void {}
  go();
}
function other(b: number): void {
  const f = (go: () => void) => go();
  f(() => {});
}
