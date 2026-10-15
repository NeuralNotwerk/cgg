package util;
public class Monitor {
  public Monitor() {}
  public Monitor(Monitor other) {}
  void check(int n) {
    if (n < 0) { throw new ISE("bad %s", n); }
    Monitor m = new Monitor();
    status(1, "x");
  }
  Object status(int code, String content) { return content; }
  Object status(int code, java.io.File content, boolean inline, Object types) { return content; }
}
