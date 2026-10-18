using System;
using System.Reflection;
public class A {
    private void ByInvokeMember() {}
    private void ByGetField() {}
    private void ByCreateDelegate() {}
    private void ByGetMethod() {}
    private void PlainHigh() {}
    public static void Main() {
        typeof(A).InvokeMember("ByInvokeMember", BindingFlags.InvokeMethod, null, new A(), null);
        typeof(A).GetField("ByGetField");
        Delegate.CreateDelegate(typeof(Action), new A(), "ByCreateDelegate");
        typeof(A).GetMethod("ByGetMethod");
    }
}
