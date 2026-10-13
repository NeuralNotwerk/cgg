package gen;

public class Nimbus {
    public static class getAll_args {
        public enum _Fields {
            A;
            public static _Fields findByThriftId(int id) { return A; }
            // Bare call inside the nested class: its own findByThriftId only.
            public static _Fields findByThriftIdOrThrow(int id) {
                _Fields fields = findByThriftId(id);
                return fields;
            }
        }
        public void unset_aze() {}
        public void setFieldValue() { unset_aze(); }
        // Positive control: overloads within the caller's own class fan out.
        public void put(int a) {}
        public void put(String a) {}
        public void callPut() { put(1); }
    }
    public static class getAll_result {
        public enum _Fields {
            B;
            public static _Fields findByThriftId(int id) { return B; }
            public static _Fields findByThriftIdOrThrow(int id) {
                _Fields fields = findByThriftId(id);
                return fields;
            }
        }
        public void unset_aze() {}
        public void setFieldValue() { unset_aze(); }
    }
}
