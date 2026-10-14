package app;

public class Main {
    public static void main(String[] args) throws Exception {
        Main.class.getDeclaredMethod("reflected").invoke(new Main());
        Main.class.getMethod("reflectedPublic").invoke(new Main());
    }

    private void reflected() {}
    public void reflectedPublic() {}
    private void notReflected() {}
}
