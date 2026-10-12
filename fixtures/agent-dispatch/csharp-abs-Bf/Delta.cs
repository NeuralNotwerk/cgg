public class Delta : Processor {
    public override void Process() { System.Console.WriteLine("Delta"); }
    public override void UnusedOp() { System.Console.WriteLine("u"); }
}
