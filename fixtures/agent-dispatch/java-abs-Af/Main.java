public class Main {
    private Processor field;
    Main(Processor p) { this.field = p; }
    void runField() { field.process(); }
    public static void main(String[] args) {
        new Main(new Alpha()).runField();
        new Main(new Beta()).runField();
    }
}
