class Holder(private val field: Processor) {
    fun runField() { field.process() }
}

fun main() {
    Holder(Alpha()).runField()
    Holder(Beta()).runField()
}
