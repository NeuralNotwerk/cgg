abstract class Base {
    fun run() {
        process()
    }
    protected abstract fun process()
    protected abstract fun unusedOp()
}
