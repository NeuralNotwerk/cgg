package p
import java.io.InputStream
class Conn(val stream: InputStream) {
    fun close() { }
    fun shutdown(other: Conn) {
        other.stream.close()
        this.stream.close()
        stream.close()
    }
}
