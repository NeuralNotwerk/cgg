using NUnit.Framework;
using Xunit;
using System.Collections.Generic;

namespace Ex.Tests
{
    [TestFixture]
    public class NTests
    {
        private static IEnumerable<int> StringCases() { yield return 1; }
        private static IEnumerable<int> NameofCases() { yield return 2; }
        private static void TestDeadHelper() { }

        [TestCaseSource("StringCases")]
        public void UsesString(int x) { }

        [TestCaseSource(nameof(NameofCases))]
        public void UsesNameof(int x) { }
    }

    public class XTests
    {
        public static IEnumerable<object[]> MemberDataSource() { yield return new object[] { 1 }; }
        private static IEnumerable<object[]> PrivMemberData() { yield return new object[] { 1 }; }

        [Theory]
        [MemberData(nameof(MemberDataSource))]
        public void UsesMember(int x) { }

        [Theory]
        [MemberData(nameof(PrivMemberData))]
        public void UsesPriv(int x) { }
    }
}
