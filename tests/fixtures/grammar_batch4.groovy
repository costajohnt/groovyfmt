// Constructs the fourth grammar batch added.
import spock.lang.Specification

class Batch4Spec extends @Marked Specification {
    private final Map<String, @Nullable String> plugins = [:]

    def "names that used to be keywords"() {
        given:
        var.set(123)
        String var = 'x'
        Pipeline pipeline = redis.pipelined()
        trait.parentFile.mkdirs()
        var count = 3

        expect:
        unresolvedVariable { var ->
            var.name
        }
    }

    def "command calls"() {
        when:
        respond new Item(name: 'a'), model: [age: 20]
        assertNull null.encodeAsBase64()
        assertTrue !err.contains('x'), err
        assertTrue set instanceof SortedSet, 'msg'
        assertEquals [:], map
        all*.exclude group: 'g', module: 'm'
        if (message instanceof String) reply message.reverse()
        else stop()

        then:
        (el searchBy).click()
        (go LoginPage).login(user,
            pass)
        if (x instanceof Foo f) {
            f.bar()
        }
    }

    def "generic values and casts"() {
        expect:
        def supplier = Mock(Supplier<Long>)
        def jobs = mapper.readValue(s, ArrayList<Job>.class)
        def task = { 1 } as java.util.concurrent.Callable<Integer>
        def kinds = [] as WatchEvent.Kind[]
        for (int i = 0; i<n; i++) {
            total += i
        }
    }

    def "anonymous classes"() {
        def r = new Runnable() {
            private int runs = 0

            @Override
            void run() {
                runs++
            }
        }
        def wrapped = new Thread(group,
            'name') {
            void run() { }
        }
        def m = new HashMap() {{
            put('a', 1)
        }}
        def cast = foo(a) \
            .bar()

        expect:
        msg =~ / X /
        text.split(/ /).size() == 2
        delta > .05D
    }
}

enum Level {
    @Deprecated
    LOW,
    HIGH() {
        String label() { 'high' }
    },
    MAX
}

interface Shape {
    default String describe() {
        "shape"
    }

    @Option flag()
}
