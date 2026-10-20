package p

import spock.lang.Specification

class BuilderSpec extends Specification {
    def builder = new Builder()

    def setup() { }

    def "builds the project"() {
        when:
        builder.build()

        then:
        noExceptionThrown()
    }

    def 'reports a failure'() {
        expect:
        builder.compile() == 1
    }
}
