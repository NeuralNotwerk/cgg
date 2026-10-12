package p;
import java.util.Map;
class Handler { Object get(String k) { return null; } int size() { return 0; } }
public class A {
    void lookup(Map<String, Handler> handlers) { handlers.get("a"); handlers.size(); }
}
