package q;

public class F {
    void go() {
        B b = new B() {
            @Override
            public void stop() { super.stop(); }
        };
        b.stop();
    }
}
