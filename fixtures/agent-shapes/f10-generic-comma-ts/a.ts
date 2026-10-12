class Handler { has(k: string): boolean { return false; } }
export function lookup(handlers: Map<string, Handler>, x: number) { handlers.has("a"); }
