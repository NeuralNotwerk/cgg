namespace App {
  public class Use {
    public void Go(Svc s) {
      s.Run();
      s.Run(1);
      s.Run(1, 2);
      s.Log("a");
      s.Twice(1);
    }
  }
}
