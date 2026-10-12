public class Main {
    static <T extends Processor> void run(T p) {
        p.process();
    }
    public static void main(String[] args) {
        run(new Alpha());
        run(new Beta());
        run(new Gamma());
        run(new Delta());
        run(new Epsilon());
        run(new Zeta());
    }
}
