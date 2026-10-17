function target(): number { return 1; }
function live(): number { return 2; }

export function main(): number {
  live();
  throw new Error("x");
  target();
}
