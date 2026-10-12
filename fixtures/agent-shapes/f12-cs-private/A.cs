namespace P {
public class A {
    private static T GenS<T>(T x) { return x; }
    private T GenI<T>(T x) { return x; }
    private static int CtlS(int x) { return x; }
    private int CtlI(int x) { return x; }
    private static T GenUnused<T>(T x) { return x; }
    public static void UseGen() {
        var a = new A();
        A.GenS<string>("x");
        a.GenI<string>("y");
    }
    public static void UseCtl() {
        var a = new A();
        A.CtlS(1);
        a.CtlI(2);
    }
}
}
