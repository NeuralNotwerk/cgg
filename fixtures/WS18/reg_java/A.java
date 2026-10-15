package p;
import javax.xml.bind.annotation.XmlType;
@XmlType(factoryClass = A.class, factoryMethod = "newA")
public class A {
  public static A newA() { return new A(); }
  private void dead() {}
  void wire(Runnable r) { }
  public static void main(String[] a) { new A().wire(A::onRun); }
  static void onRun() {}
}
