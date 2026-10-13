package p;

import java.util.function.Supplier;
import org.thirdparty.Handler;

class Ext extends Handler {
    // overrides an out-of-graph base member: the runtime calls it
    @Override
    void handle(int code) { helper(); }
    private void helper() {}
    // package-private method with no marker, in a class with external base
    void noMarker() {}
    void callsSuper() { super.start(); }
}

class Sup implements Supplier<String> {
    @Override
    public String get() { return "x"; }
}

class Untyped {
    // receiver is a variable whose type cgg does not know
    void run(Object orderService) { }
    private void process() {}
    private void unrelated() {}
    void driver(Untyped p) {
        Object q = make();
        q.process();
    }
    private static Object make() { return null; }
}

class Starter {
    void start() {}
}
