class P { constructor(x) { this.x = x; } }
class K extends P { constructor() { super(1); } }
function main() { return new K(); }
