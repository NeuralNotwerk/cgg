package demo;

import java.io.IOException;
import java.io.ObjectInputStream;
import java.io.ObjectOutputStream;
import java.io.ObjectStreamException;
import java.io.Serializable;

class Rec implements Serializable {
    private void readObject(ObjectInputStream in) throws IOException, ClassNotFoundException { in.defaultReadObject(); }
    private void writeObject(ObjectOutputStream out) throws IOException { out.defaultWriteObject(); }
    private void readObjectNoData() throws ObjectStreamException {}
    private Object readResolve() { return this; }
    private Object writeReplace() { return this; }
}

class NotSer {
    // near miss: same name, not the serialization signature
    private Object readObject(String path) { return null; }
}
