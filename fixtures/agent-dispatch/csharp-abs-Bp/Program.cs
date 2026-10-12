public class Program {
    static void Run(Processor p) {
        p.Process();
    }
    public static void Main(string[] args) {
        Run(new Alpha());
        Run(new Beta());
        Run(new Gamma());
        Run(new Delta());
        Run(new Epsilon());
        Run(new Zeta());
    }
}
