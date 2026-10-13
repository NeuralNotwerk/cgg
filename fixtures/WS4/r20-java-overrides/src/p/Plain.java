package p;

// Near miss: an inherited method no subclass overrides still binds.
public class Plain extends PlainBase {
    void run() { helper(1); helper("x"); }
}
