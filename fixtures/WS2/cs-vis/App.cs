using System;

namespace P {
public class App {
    public static void Main(string[] args) { new App().Entry(); }

    void Entry() { }

    // B1 pattern: attribute with arguments before an explicit modifier.
    [Obsolete("x")]
    public void AnnotatedPublicUnused() { }

    [Obsolete("x", false)]
    protected void AnnotatedProtectedUnused() { }

    [Obsolete("x")] internal void AnnotatedInternalUnused() { }

    // Near-miss: attribute args then private -> private.
    [Obsolete("x")]
    private void AnnotatedPrivateUnused() { }

    // Near-miss: attribute args, no modifier -> private (class default).
    [Obsolete("x")]
    void AnnotatedDefaultUnused() { }

    // Near-miss: a parameter named publicId is not a modifier.
    void TakesPublicId(string publicId) { }

    protected internal void ProtInternalUnused() { }
    private protected void PrivProtUnused() { }

    static App() { }

    public int Prop { get; private set; }
}

public interface IRepo {
    // Implicitly public (C# interface members).
    void FindAll();
    [Obsolete("x")]
    void AnnotatedIfaceUnused();
    // C# 8 default interface method with an explicit private modifier.
    private void PrivateHelperUnused() { }
    protected void ProtectedIfaceUnused() { }
    // Default implementation, no modifier: public.
    void DefaultImplUnused() { }
    int Count { get; }
}

public struct S {
    // Struct members default to private.
    void StructDefaultUnused() { }
}

public record R(int X) {
    void RecordDefaultUnused() { }
}

class Impl : IRepo {
    public void FindAll() { }
    // Explicit interface implementation: no modifier allowed.
    void IRepo.AnnotatedIfaceUnused() { }
    public int Count => 0;
}
}
