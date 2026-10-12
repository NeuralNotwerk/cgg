public abstract class Base {
    public final void run() {
        process();
    }
    protected abstract void process();
    protected abstract void unusedOp();
}
