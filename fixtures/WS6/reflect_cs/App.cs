using System;
using System.Reflection;

namespace App
{
    public class Program
    {
        public static void Main(string[] args)
        {
            typeof(Program).GetMethod("Reflected", BindingFlags.NonPublic | BindingFlags.Instance).Invoke(new Program(), null);
        }

        private void Reflected() {}
        private void NotReflected() {}
    }
}
