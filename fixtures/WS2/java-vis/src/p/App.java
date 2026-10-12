package p;

public class App {
    public static void main(String[] args) {
        new App().entry();
    }

    void entry() { }

    // B1 pattern: annotation with arguments before an explicit modifier.
    @SuppressWarnings("unchecked")
    public void annotatedPublicUnused() { }

    @Deprecated(since = "1", forRemoval = false)
    protected void annotatedProtectedUnused() { }

    // Near-miss: annotation args, then private -> must stay private.
    @SuppressWarnings("unused")
    private void annotatedPrivateUnused() { }

    // Near-miss: annotation args, no modifier -> package-private.
    @SuppressWarnings("unused")
    void annotatedPackageUnused() { }

    // Near-miss: a parameter named publicId is not a modifier.
    void takesPublicId(String publicId) { }

    private void plainPrivateUnused() { }

    protected void plainProtectedUnused() { }
}

interface Repo {
    // Implicitly public (JLS 9.4).
    void findAll();
    default void defaultMethodUnused() { }
    static void staticMethodUnused() { }
    // Java 9+ private interface method: still private.
    private void privateHelperUnused() { }
    // Annotation arguments on an interface member.
    @SuppressWarnings("x")
    void annotatedIfaceUnused();
}

enum Color {
    RED, GREEN;
    // Enum methods follow class rules: package-private.
    void enumPackageUnused() { }
    public void enumPublicUnused() { }
}

record Point(int x, int y) {
    // Record methods follow class rules too.
    void recordPackageUnused() { }
}

class Outer {
    interface Inner {
        // Nested interface: still implicitly public.
        void innerIfaceUnused();
    }
    void outerPackageUnused() {
        // Anonymous class inside a class: class rules.
        Runnable r = new Runnable() {
            public void run() { }
            void anonPackageUnused() { }
        };
    }
}

interface WithAnon {
    default void make() {
        // A class body nested inside an interface follows class rules.
        Object o = new Object() {
            void anonInIfaceUnused() { }
        };
    }
}
