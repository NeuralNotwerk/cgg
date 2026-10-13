package r;

// Near miss: same-named methods of an unrelated class are never reached.
public class Other {
    public void testQuery(String sql) {}
    public void add(StringBuilder sb, String name, String val) {}
    public void aggregators(String a) {}
}
