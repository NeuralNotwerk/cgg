public class Main {
    static void run(Processor p) {
        p.process();
    }
    public static void main(String[] args) {
        run(new Alpha());
        run(new Beta());
    }
}
