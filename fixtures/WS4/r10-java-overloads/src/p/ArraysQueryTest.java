package p;

import static q.CookieUtil.*;

public class ArraysQueryTest extends BaseQueryTest {
    // An overloaded method of the base class: the whole overload set.
    public void testArrays() {
        testQuery("select 1");
        aggregators("x");
    }

    // `import static q.CookieUtil.*`: CookieUtil's overload set of `add`.
    public void encode(StringBuilder buf) {
        add(buf, "name", "value");
    }
}
