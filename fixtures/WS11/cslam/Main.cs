using System;
using System.Collections.Generic;
namespace M {
    public class Main {
        public void AClosure(List<Foo> xs) {
            xs.ForEach(x => x.Run());
        }
        public void BTyped(List<Foo> xs) {
            xs.ForEach((Foo x) => x.Run());
        }
        public void Each(Action<Foo> cb) {}
        public void CCallback() {
            Each(x => x.Run());
        }
        public void DBlock(List<Foo> xs) {
            xs.ForEach(x => { x.Run(); });
        }
    }
}
