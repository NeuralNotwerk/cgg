namespace P {
public class A {
    public static T GenS<T>(T x) { return x; }
    public T GenI<T>(T x) { return x; }
    public static T GenBare<T>(T x) { return x; }
    public static int CtlS(int x) { return x; }
    public int CtlI(int x) { return x; }
    public static void UseGen() {
        var a = new A();
        A.GenS<string>("x");
        a.GenI<string>("y");
        GenBare<int>(3);
    }
    public static void UseCtl() {
        var a = new A();
        A.CtlS(1);
        a.CtlI(2);
    }
}
}
