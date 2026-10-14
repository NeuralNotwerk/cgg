public record A(int x) {
  static A of(int x) { return new A(x); }
  public int twice() { return x * 2; }
  public String describe() { return "static " + twice(); }
}
