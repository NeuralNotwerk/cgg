using System.Collections;
using System.Collections.Generic;
using NUnit.Framework;

namespace Demo
{
    public class NTests
    {
        [TestCaseSource(nameof(Cases))]
        public void ByNameof(int x) { }

        [TestCaseSource("StringCases")]
        public void ByString(int x) { }

        [TestCaseSource(typeof(OtherSource), nameof(OtherSource.Remote))]
        public void ByType(int x) { }

        private static IEnumerable<int> Cases() { yield return 1; }
        private static IEnumerable<int> StringCases() { yield return 2; }
        // near miss
        private static IEnumerable<int> UnusedCases() { yield return 3; }
    }

    internal class OtherSource
    {
        internal static IEnumerable<int> Remote() { yield return 4; }
    }
}
