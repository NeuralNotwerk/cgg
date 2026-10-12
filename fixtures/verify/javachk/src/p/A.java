package p;
import java.util.List;
public class A {
    public static void main(String[] args) {
        List<String> xs = List.of("a");
        xs.forEach(A::show);
        new A().run(xs);
    }
    private static void show(String s) { System.out.println(s); }
    private static void neverUsed(String s) { }
    void run(List<String> xs) { Repo r = make(); r.findAll(); }
    private Repo make() { return null; }
    @SuppressWarnings("unchecked")
    public void annotatedPublic() { }
}
interface Repo { void findAll(); void unusedOp(); }
