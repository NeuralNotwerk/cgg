class P { constructor(x: number) { } }
class K extends P { constructor() { super(1); } }
export function main() { return new K(); }
