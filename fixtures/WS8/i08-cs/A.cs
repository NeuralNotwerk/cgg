namespace N {
  public class Base { public void Run() {} }
  public class Alpha : Base { }
  public class Gamma { public void Run() {} }
  public class User {
    public void Go() {
      new Alpha().Run();
      var a = new Alpha();
      a.Run();
    }
  }
}
