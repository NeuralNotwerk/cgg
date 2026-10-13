package p;

public abstract class ActionSet {
    // A bare call to an abstract method its nested subclasses override:
    // a virtual call with several bodies. Refused (recorded), so the
    // overrides keep the doubt of an unresolved same-named site.
    public ActionSet add(int a) {
        if (a > 0) {
            return addOne(a);
        }
        return addOne(a + 1);
    }
    abstract ActionSet addOne(int a);
    abstract ActionSet addAll(int a);

    private static class EmptySet extends ActionSet {
        @Override
        ActionSet addOne(int a) { return this; }
        @Override
        ActionSet addAll(int a) { return this; }
    }
    private static class Single extends ActionSet {
        @Override
        ActionSet addOne(int a) { return this; }
        @Override
        ActionSet addAll(int a) { return this; }
    }
}
