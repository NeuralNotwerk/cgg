using System;
using System.Collections.Generic;
using System.Linq;

namespace App
{
    public class Tool : IDisposable
    {
        public void Work() { }
        public void Dispose() { }
    }

    public class Program
    {
        // Parameter.
        static void Use(Action next) { next(); }

        // foreach variable.
        static void Each(List<Action<int>> hs) { foreach (var handler in hs) handler(1); }

        // Pattern variable, in scope after the if.
        static void Pattern(object o)
        {
            if (!(o is Action callback)) return;
            callback();
        }

        // out var.
        static void Out()
        {
            Try(out Action step);
            step();
        }

        // Lambda parameter.
        static void Lambda(List<Func<int>> fs)
        {
            var total = fs.Sum(factory => factory());
        }

        // Local function: the call is the local function.
        static int Local(int x)
        {
            int twice(int v) => v * 2;
            return twice(x);
        }

        static void Try(out Action a) { a = null; }

        // Same-class methods named like the locals above.
        static void next() { }
        static void handler(int x) { }
        static void callback() { }
        static void step() { }
        static int factory() { return 0; }
        static int twice(int v) { return v; }

        static Tool Make() { return null; }

        // Positive controls: a typed parameter, and locals typed from a
        // call's declared return type (plain, `using var`, in a block).
        static void Control(Other other)
        {
            other.Done();
            var tool = Make();
            tool.Work();
            using var held = Make();
            held.Work();
            {
                var inner = Make();
                inner.Work();
            }
        }

        public static void Main()
        {
            Use(null);
            Each(null);
            Pattern(null);
            Out();
            Lambda(null);
            Local(1);
            Control(null);
        }
    }
}
