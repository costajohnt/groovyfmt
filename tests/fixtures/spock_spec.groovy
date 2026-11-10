import spock.lang.Specification

class StackSpec extends Specification {
    def 'push then pop returns the item'() {
        given:
        def stack = new ArrayDeque()

        when:
        stack.push(item)

        then:
        stack.pop() == item

        where:
        item << [1, 'two', null]
    }

    def 'maximum of two numbers'() {
        expect:
        Math.max(a, b) == c

        where:
        a | b | c
        1 | 3 | 3
        7 | 4 | 7
    }
}
