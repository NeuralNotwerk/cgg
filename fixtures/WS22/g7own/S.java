class Sup {
  abstract static class It {
    void find() { boolean b = next(1); }
    abstract boolean next(int v);
  }
  It a() { return new It() { boolean next(int v) { return true; } }; }
  It b() { return new It() { boolean next(int v) { return false; } }; }
}
