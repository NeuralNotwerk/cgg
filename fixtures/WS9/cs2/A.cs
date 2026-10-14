namespace N {
public class B { public void Plain() {} public B Next() { return this; } public B Child; }
public class C {
  public void Caller(B b, B? n) {
    b?.Plain();
    n.Plain();
    b?.Child.Plain();
    b?.Next().Plain();
  }
}
}
