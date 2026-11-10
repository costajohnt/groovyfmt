class AdderSpec extends spock.lang.Specification {
    def 'adds'() {
        given:
            def adder = new Adder()
        when:
            def r = adder.add(x, y)
        then:
            r == sum
        where:
            x | y || sum
            1 | 2 || 3
            0 | 0 || 0
    }
}
