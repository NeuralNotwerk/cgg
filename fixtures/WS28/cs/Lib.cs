namespace App {
  public class Svc {
    public void Run() {}
    public void Run(int a) {}
    public void Run(int a, int b, int c = 0) {}
    public void Log(params object[] xs) {}
    public void Log(string s) {}
  }
  public static class Ext {
    public static int Twice(this Svc s, int n) { return n; }
    public static int Twice(this Svc s, int n, int m) { return n; }
  }
}
