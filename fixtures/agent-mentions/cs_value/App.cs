using System;
using System.Linq;
using System.Threading;
using System.Collections.Generic;

namespace Ex
{
    public class App
    {
        public event EventHandler Changed;

        private static int SelectTarget(int x) { return x + 1; }
        private void EventTarget(object s, EventArgs e) { }
        private void ThreadTarget() { }
        private static bool WhereTarget(int x) { return x > 0; }
        private int FuncAssigned() { return 1; }
        private void ActionArg() { }
        private void ReflectTarget() { }
        private void NameofOnly() { }
        private void DeadControl() { }
        private void CalledControl() { }

        public void Run(List<int> xs)
        {
            var ys = xs.Select(SelectTarget).Where(WhereTarget).ToList();
            Changed += EventTarget;
            new Thread(ThreadTarget).Start();
            Func<int> f = FuncAssigned;
            Invoke(ActionArg);
            GetType().GetMethod("ReflectTarget");
            Console.WriteLine(nameof(NameofOnly));
            CalledControl();
        }

        private void Invoke(Action a) { a(); }

        public static void Main() { new App().Run(new List<int>()); }
    }
}
