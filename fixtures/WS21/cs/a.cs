namespace App {
  class Base {
    public Base(int x) { }
  }
  class P : Base {
    public P() : this(1) { }
    public P(int y) : base(y) { }
  }
  class Q {
    void Configure() {
      var p = new P();
      var q = new P(2);
      Helper h = new();
    }
  }
  class Helper {
    public Helper() { }
  }
}
