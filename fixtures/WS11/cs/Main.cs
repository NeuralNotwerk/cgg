using System;
namespace M {
    public class Main {
        public void AControl(Foo x) {
            x.Run();
        }
        public void BNullable(Foo? x) {
            x?.Run();
        }
        public void CNullableBang(Foo? x) {
            x!.Run();
        }
        public void DWrapped(Nullable<Pt> x) {
            x?.Run();
        }
        public void EStructNullable(Pt? x) {
            x?.Run();
        }
        public void FLocal() {
            Foo? y = null;
            y?.Run();
        }
        public void GLocalPlain() {
            Foo y = new Foo();
            y.Run();
        }
        private Foo? f;
        private Foo g;
        public void M1() {
            f?.Run();
        }
        public void M2() {
            g.Run();
        }
        public Foo? Mk() { return null; }
        public void HRet() {
            var foo = Mk();
            foo?.Run();
        }
        public void IUntyped(dynamic x) {
            x.Run();
        }
    }
}
