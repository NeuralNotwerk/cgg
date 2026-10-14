namespace N {
  public class E {
#if HAVE_ASYNC
    public async Task<int> Load(int x)
#else
    public int Load(int x)
#endif
    {
      return Helper(x);
    }
    private int Helper(int x) { return x; }
  }
  public partial class Data {
    partial void OnLoaded();
    partial void OnSaved() => Log();
    void Log() {}
  }
}
