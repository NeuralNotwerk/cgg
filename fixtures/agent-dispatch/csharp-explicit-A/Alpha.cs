public class Alpha : IProcessor {
    void IProcessor.Process() { System.Console.WriteLine("Alpha"); }
    void IProcessor.UnusedOp() { System.Console.WriteLine("u"); }
}
