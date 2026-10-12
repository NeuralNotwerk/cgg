public class Delta implements Processor {
    @Override
    public void process() { System.out.println("Delta"); }
    @Override
    public void unusedOp() { System.out.println("u"); }
}
