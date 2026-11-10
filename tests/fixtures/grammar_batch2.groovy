import groovy.transform.Field

@Field String STEP_NAME = 'build'
@Field final Map CONFIG = [:]
@Field def counter = 0

bind Foo, Bar
attributes 'Main-Class': main, 'Archive': 'true'
script.writeFile file: 'out.txt', text: content
helper.registerAllowedMethod 'sh', [Map], { m ->
    m.script
}
task copy {
    from 'src'
}
log.warn '*' * 80

def bytes = new byte[10]
def grid = new int[2][3]
def names = new String[] {
    'a',
    'b'
}

def inc = x -> x + 1
def add = (a, b) -> {
    a + b
}
def trimmed = names.stream()
    .map(s -> s.trim())
    .filter((String n) -> n.length() > 0)

def (String first, int second) = ['a', 1]
(first, second) = ['b', 2]
assert second == 2, 'second should be 2'

class Spec {
    @Shared log = []
    @Inject Engine engine // the engine

    def static helper() {
        synchronized (this) {
            log << 'x'
        }
    }

    void adapt(Object t, boolean initializing) {
        Neo4jSession session = (Neo4jSession) this.session
        if (initializing) {
            List itemIds = (List) items.inventoryItemId
            while (itemIds) {
                itemIds = null
            }
        }
        def value = (String) t.name
        return value
    }

    void fill(Map execMap, Object e) {
        execMap.user = (e.user)
        execMap.'date-started' = [time: e.started]
        v = (Map) e.data
        bar = 1
        assertEquals 'v', e.value
        // a comment after a command call
        assert bar
    }

    def <T> T withContext(Closure<T> block) {
        block()
    }
}

@interface Meta {
    String value() default 'x'
}
