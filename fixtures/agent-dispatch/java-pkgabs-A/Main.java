public class Main {
    static void run(Base p) {
        p.process();
    }
    public static void main(String[] args) {
        run(new Alpha());
        run(new Beta());
    }
}
