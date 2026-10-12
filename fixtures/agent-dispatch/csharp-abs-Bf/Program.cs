public class Program {
    private Processor field;
    Program(Processor p) { field = p; }
    void RunField() { field.Process(); }
    public static void Main(string[] args) {
        new Program(new Alpha()).RunField();
        new Program(new Beta()).RunField();
    }
}
