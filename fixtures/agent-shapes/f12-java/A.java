package p;
public class A {
    static <T> T genS(T x) { return x; }
    <T> T genI(T x) { return x; }
    static int ctlS(int x) { return x; }
    int ctlI(int x) { return x; }
    public static void useGen() {
        A a = new A();
        A.<String>genS("x");
        a.<String>genI("y");
    }
    public void useGenThis() {
        this.<String>genI("z");
    }
    public static void useCtl() {
        A a = new A();
        A.ctlS(1);
        a.ctlI(2);
    }
}
