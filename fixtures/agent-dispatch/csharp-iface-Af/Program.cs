public class Program {
    private IProcessor field;
    Program(IProcessor p) { field = p; }
    void RunField() { field.Process(); }
    public static void Main(string[] args) {
        new Program(new Alpha()).RunField();
        new Program(new Beta()).RunField();
    }
}
