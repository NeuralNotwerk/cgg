function outer(a: number): void {
  function helper(): void {}
  helper();
}
function other(b: number): void {
  helper();
}
