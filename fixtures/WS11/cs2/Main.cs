namespace M {
    public class Main {
        public void FLocal() {
            Foo? zz = null;
            zz?.Run();
        }
        public void GLocal() {
            Bar zq = null;
            zq.Run();
        }
    }
}
