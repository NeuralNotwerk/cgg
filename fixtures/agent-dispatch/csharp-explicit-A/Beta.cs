public class Beta : IProcessor {
    void IProcessor.Process() { System.Console.WriteLine("Beta"); }
    void IProcessor.UnusedOp() { System.Console.WriteLine("u"); }
}
