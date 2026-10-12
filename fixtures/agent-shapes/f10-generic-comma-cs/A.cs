using System.Collections.Generic;
namespace P {
class Handler { public bool ContainsKey(string k) { return false; } }
public class A {
    void Lookup(Dictionary<string, Handler> handlers) { handlers.ContainsKey("a"); }
}
}
