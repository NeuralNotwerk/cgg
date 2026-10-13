using System;
using Some.External;

namespace App
{
    interface IProcessor { void Process(); }

    abstract class Handler
    {
        protected abstract void Run();
        internal abstract void Step();
        void Unused() { }
        public void Go() { Run(); Step(); }
    }

    class Concrete : Handler
    {
        protected override void Run() { }
        internal override void Step() { Helper(); }
        void Helper() { }
    }

    class Proc : IProcessor
    {
        void IProcessor.Process() { }
    }

    class ExtImpl : IDisposable, IExternalThing
    {
        void IDisposable.Dispose() { }
        void IExternalThing.Tick() { }
    }

    class Widget : ExternalBase
    {
        protected override void OnPaint() { base.OnPaint(); }
        internal override void Layout() { base.Layout(); }
    }

    class Other
    {
        internal void Layout() { }
        internal void OnPaint() { }
    }

    public static class Program
    {
        public static void Main()
        {
            IProcessor p = new Proc();
            p.Process();
            Handler h = new Concrete();
            h.Go();
        }
    }
}
