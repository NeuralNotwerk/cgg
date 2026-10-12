public class Beta implements Processor {
    @Override
    public void process() { System.out.println("Beta"); }
    @Override
    public void unusedOp() { System.out.println("u"); }
}
