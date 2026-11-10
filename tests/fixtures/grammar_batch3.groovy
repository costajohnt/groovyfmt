// Constructs the third grammar batch added, and line breaks it now reads as
// statement boundaries.
import spock.lang.Specification

@Scaffold(Foo<User>)
class ArraySpec extends Specification {
    static @Rule TestName name = new TestName()
    final @Delegate Map target = [:]
    private static @Nullable String label() { null }
    def String plain
    def Closure<?> body

    def "array types are values"() {
        expect:
        bean(String[], 'x').class == String[]
        byte[].class.simpleName == 'byte[]'
        list.toArray(String[]::new).size() == 2
        map['a', 'b'] == [1, 2]

        where:
        type     | size
        List[]   | 1
        int[][]  | 2
    }

    private "a feature with a modifier"() {
        given:
        def text = "// not a comment ${name.replace 'a', 'b'} ${-> lazy}"
        def call = { a, b = 1 -> a + b }
        def spread = f(*: options)

        expect:
        call(1) == 2
    }
}

public @interface Marker {
    String value() default 'x'
}

enum Op implements Named {
    PLUS {
        int apply(int a, int b) { a + b }
    },
    NONE {}
}

def r = script.sh returnStdout: true,
    script: 'git rev-parse HEAD'
Map result = run service: 'createThing',
    with: parameters
x = foo bar
def kind = switch (r) {
    case 1, 2 -> 'low'
    default -> 'other'
}

record { entry() }
record << 'x'

Actor waitingRoom
waitingRoom = group.actor {
    loop()
}
tasks.named('validatePlugins') {
    classes.setFrom preparePluginValidationClasses
    enableStricterValidation = true
}
if (value == null) namespaces.remove key
else namespaces[key] = value
if (!found) return
done()
List<String> names
names = ['a']
items.each {
    item ->
    println item
}
