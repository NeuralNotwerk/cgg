package app

import org.junit.jupiter.params.ParameterizedTest
import org.junit.jupiter.params.provider.MethodSource
import junit.framework.TestCase

class KTest {
    @ParameterizedTest
    @MethodSource("provider")
    fun p(x: Int) {}

    companion object {
        @JvmStatic
        fun provider() = listOf(1, 2)
        @JvmStatic
        fun unusedSource() = listOf(3)
    }
}

class Legacy : TestCase() {
    fun testLegacy() {}
    fun helperNotTest() {}
}
