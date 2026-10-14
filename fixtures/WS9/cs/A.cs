namespace N {
public class B {
  public T M<T>(T x) { return x; }
  public void Plain() {}
  public static T S<T>(T x) { return x; }
}
public class C {
  public void Caller(B b, B? x) {
    b.M<string>("a");
    B.S<int>(1);
    x?.Plain();
    b.Plain();
    G<int>(2);
  }
  T G<T>(T x) { return x; }
}
}
