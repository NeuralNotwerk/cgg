namespace N {
  public class D {
    public bool ContainsKey(int key)
    {
        if (_a != null)
        {
            return Foo(key);
        }
#if HAVE_X
        else if (_b != null)
        {
            return Bar(key);
        }
#endif
        else
        {
            return Baz(key);
        }
    }
    bool Foo(int k) { return true; }
    bool Bar(int k) { return true; }
    bool Baz(int k) { return true; }
  }
}
