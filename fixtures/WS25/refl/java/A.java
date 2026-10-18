import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
public class A {
    private int byField;
    private void byFindVirtual() {}
    private void byGetMethod() {}
    private static void byFindStatic() {}
    private void plainHigh() {}
    public static void main(String[] args) throws Throwable {
        A.class.getDeclaredField("byField");
        A.class.getMethod("byGetMethod");
        MethodHandles.lookup().findVirtual(A.class, "byFindVirtual", MethodType.methodType(void.class));
        MethodHandles.lookup().findStatic(A.class, "byFindStatic", MethodType.methodType(void.class));
    }
}
