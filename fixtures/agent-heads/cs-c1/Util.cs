namespace Acme.Util {
  public static class Path {
    public static string Combine(string a, string b) { return a + b; }
  }
  public class Store {
    public string ReadAllText(string p) { return p; }
    public string SerializeObject(object o) { return ""; }
  }
}
