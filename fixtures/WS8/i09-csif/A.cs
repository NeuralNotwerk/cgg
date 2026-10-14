namespace N {
  public partial class P {
    partial void OnChanged();
    public void Run() { OnChanged(); Helper(1); }
#if HAVE_ASYNC
    public void Helper(int x) {
      Other();
    }
#else
    public void Helper(int x) {
      Other();
      switch (x) { case 1: break; default: break; }
    }
#endif
    private void Other() {}
  }
}
