public class M {
    private static int target() { return 1; }
    private static int live() { return 2; }

    public static void main(String[] args) {
        live();
        return;
        // javac would reject the next line; the extractor does not care
    }

    private static int other(int x) {
        while (true) {
            break;
            target();
        }
        return 0;
    }
}
