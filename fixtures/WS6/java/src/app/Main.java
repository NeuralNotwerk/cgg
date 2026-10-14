package app;

import java.util.List;
import java.util.function.Supplier;

public class Main {
    public static void main(String[] args) {
        Main m = new Main();
        m.run(List.of("a"));
    }

    void run(List<String> xs) {
        xs.forEach(this::onItem);            // this::m
        xs.stream().map(Main::upper).count(); // Type::static
        new Thread(this::worker).start();     // new Thread(this::m)
        Supplier<Helper> s = Helper::new;     // Type::new
        s.get();
        Helper h = new Helper();
        xs.forEach(h::consume);               // var::m (typed)
    }

    private void onItem(String s) {}
    private static String upper(String s) { return s; }
    private void worker() {}
    // near miss: same names, never referenced
    private void unusedSibling() {}
}

class Helper {
    Helper() {}
    void consume(String s) {}
    private void neverReferenced() {}
}

class Other {
    // same simple name as Main.onItem, not referenced anywhere: must stay HIGH
    private void onItem(String s) {}
}

class Base {
    void hook() {}
}

class Derived extends Base {
    void go(List<String> xs) {
        xs.forEach(x -> {});
        Runnable r = super::hook;            // super::m
        r.run();
    }
}
