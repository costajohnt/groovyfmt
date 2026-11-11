package org.example.wrapping.fixture.with.a.pkg.name.that.is.long.enough.to.overflow.it

import org.example.wrapping.fixture.with.an.imported.name.that.is.long.enough.to.go.Name

@SuppressWarnings(['UnusedVariable', 'UnnecessaryGString', 'MethodSize', 'AbcMetric'])
class WrapFixture {
    def build(List<String> names, Map<String, Object> options) {
        def result = someService.callSomething(argumentOne, argumentTwo, argumentThree, four)
        def builder = Foo.builder().name("alpha").value(computeTheValue(a, b)).tags(tags).build()
        if (someCondition && otherCondition || yetAnotherCondition && theLastCondition || done) {
            println "a GString ${foo(alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota)} stays"
        }
        def settings = [alpha: 1, beta: [2, 3, 4], gamma: 'three', delta: foo(x, y), epsilon: 5]
        String s = "a very long string literal that cannot be broken at all because it is just long"
        foo(a, b) // a long trailing comment that makes the line long enough to want wrapping
        implementation group: 'org.something', name: 'artifact-name', version: '1.2.3-SNAPSHOT'
        names.each { name -> registry.register(name, options.get(name), defaults, overrides, flags) }
        def text = '''
            lines inside a multi-line string are never wrapped, however long they are, not even this
        '''
        task(type: Copy, description: 'copies the things to the place', group: 'build') {
            from(sourceDirectory, otherSourceDirectory, yetAnotherSourceDirectory, four)
        }
        return computeTheFirstThing(argumentOne) + computeTheSecondThing(argumentTwo) + more
    }

    def 'a spock feature'() {
        expect:
        computeTheFirstThing(argumentOne, argumentTwo) == expectedResultOfTheComputation
        where:
        argumentOne                           | argumentTwo                         | expectedResultOfTheComputation
        [alpha, beta, gamma, delta, epsilon]  | [zeta, eta, theta, iota, kappa]     | 1
    }

    def choose(boolean condition) {
        def value = condition ? computeTheFirstThing(argumentOne) : computeTheSecondThing(two)
        def other = configuredValueFromTheEnvironment ?: defaultValueFromTheConfigurationFile
        switch (value) {
            case [alphaAlphaAlpha, betaBetaBeta, gammaGammaGamma, deltaDeltaDelta, epsilon]:
                return value
        }
    }
}
