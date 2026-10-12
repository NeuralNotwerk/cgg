package p;
import java.io.InputStream;
class Conn {
    InputStream stream;
    void close() { }
    void shutdown(Conn other) {
        other.stream.close();
        this.stream.close();
        stream.close();
        new Conn().close();
        other.self().close();
    }
    Conn self() { return this; }
}
