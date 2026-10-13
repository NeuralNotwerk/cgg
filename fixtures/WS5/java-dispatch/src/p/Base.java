package p;

abstract class PropertyValidator {
    abstract String getMember(int x);
    // never called, never overridden: a correct HIGH dead finding (control)
    void unusedHelper() {}
}

final class InputValidator extends PropertyValidator {
    @Override
    String getMember(int x) { return format(x); }
    private String format(int x) { return "" + x; }
}

class OutputValidator extends PropertyValidator {
    String getMember(int x) { return "o"; }
}

interface Shape { void draw(); }

class Square implements Shape {
    public void draw() {}
}

class Middle extends PropertyValidator {}

class Leaf extends Middle {
    String getMember(int x) { return "leaf"; }
}

public class Base {
    public static void main(String[] args) {
        PropertyValidator v = pick(args.length);
        v.getMember(1);
        Shape s = new Square();
        s.draw();
    }
    private static PropertyValidator pick(int n) { return n > 0 ? new InputValidator() : new OutputValidator(); }
}
