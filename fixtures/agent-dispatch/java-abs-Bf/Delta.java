public class Delta extends Processor {
    @Override
    public void process() { System.out.println("Delta"); }
    @Override
    public void unusedOp() { System.out.println("u"); }
}
