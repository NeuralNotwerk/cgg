package gen;

// Near miss: a method of an ENCLOSING class is reachable bare.
public class Outer {
    static int helper() { return 1; }
    public static class Inner {
        int use() { return helper(); }
    }
}
