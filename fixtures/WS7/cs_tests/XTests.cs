using System.Collections;
using System.Collections.Generic;
using Xunit;

namespace Demo
{
    public class XTests
    {
        [Theory]
        [MemberData(nameof(Data))]
        public void Member(int x) { }

        [Theory]
        [MemberData(nameof(Ext.ExtData), MemberType = typeof(Ext))]
        public void External(int x) { }

        [Theory]
        [ClassData(typeof(Gen))]
        public void Cls(int x) { }

        private static IEnumerable<object[]> Data() { yield return new object[] { 1 }; }
        // near miss
        private static IEnumerable<object[]> Other() { yield return new object[] { 1 }; }
    }

    internal static class Ext
    {
        internal static IEnumerable<object[]> ExtData() { yield return new object[] { 1 }; }
    }

    internal class Gen : IEnumerable<object[]>
    {
        public IEnumerator<object[]> GetEnumerator() { yield return new object[] { 1 }; }
        IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();
    }
}
