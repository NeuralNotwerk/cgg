public class Program {
    static void Go(Base b) {
        b.Run();
    }
    public static void Main(string[] args) {
        Go(new Alpha());
        Go(new Beta());
    }
}
