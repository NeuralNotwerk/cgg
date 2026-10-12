package ex

import org.junit.jupiter.params.provider.MethodSource

class App {
    private fun mapTarget(x: Int): Int = x + 1
    private fun deadControl() {}
    private fun calledControl() {}
    fun run(xs: List<Int>) {
        xs.map(::mapTarget)
        xs.forEach(this::refTarget)
        calledControl()
    }
    private fun refTarget(x: Int) {}
}

private fun topLevelRef(x: Int) = x
fun useTop(xs: List<Int>) = xs.map(::topLevelRef)

class KTest {
    companion object {
        @JvmStatic
        private fun provider() = listOf(1, 2)
    }
    @org.junit.jupiter.params.ParameterizedTest
    @MethodSource("provider")
    fun t(x: Int) {}
}
