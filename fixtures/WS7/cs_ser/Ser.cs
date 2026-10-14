using System.Runtime.Serialization;

namespace Demo
{
    public class Ser
    {
        [OnDeserialized]
        private void AfterLoad(StreamingContext c) { }
        [OnDeserializing]
        private void BeforeLoad(StreamingContext c) { }
        [OnSerialized]
        private void AfterSave(StreamingContext c) { }
        [OnSerializing]
        private void BeforeSave(StreamingContext c) { }
        // near miss
        private void AfterLoadLater(StreamingContext c) { }

        public static void Main() { new Ser(); }
    }
}
