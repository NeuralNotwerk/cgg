namespace P {
public class A {
    private A next;
    private int Hop() { return 1; }
    public int Walk() { return next?.Hop() ?? 0; }
}
}
