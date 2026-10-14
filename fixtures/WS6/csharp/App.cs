using System;
using System.Linq;
using System.Collections.Generic;

namespace App
{
    public class Program
    {
        public static void Main(string[] args)
        {
            var p = new Program();
            p.Run(new List<int> { 1 });
        }

        public event EventHandler Changed;

        internal void Run(List<int> xs)
        {
            var ys = xs.Select(Double).ToList();          // method group arg
            Changed += OnChanged;                           // event +=
            var h = new EventHandler(this.OnOther);         // delegate ctor
            Func<int, int> f = Triple;                      // Func f = F
            Console.WriteLine(nameof(Named));               // nameof(F)
            Action a = Program.StaticTarget;                // Type.M
            int local = 3;
            Console.WriteLine(local);
        }

        private int Double(int x) { return x * 2; }
        private void OnChanged(object s, EventArgs e) {}
        private void OnOther(object s, EventArgs e) {}
        private int Triple(int x) { return x * 3; }
        private void Named() {}
        private static void StaticTarget() {}
        // near miss: never referenced
        private void UnusedSibling() {}
    }

    internal class Other
    {
        // same simple name as Program.OnChanged, never referenced: must stay HIGH
        private void OnChanged(object s, EventArgs e) {}
    }
}
