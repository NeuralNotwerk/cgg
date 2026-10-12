public class Program {
    static void Run(IProcessor p) {
        p.Process();
    }
    public static void Main(string[] args) {
        Run(new Alpha());
        Run(new Beta());
    }
}
