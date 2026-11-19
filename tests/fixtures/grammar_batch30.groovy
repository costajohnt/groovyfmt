// Constructs the thirtieth grammar batch added (#256): a command call whose first argument is a
// spread, attribute, pointer or reference path; a dollar-slashy string ending in a slash; the
// Groovy 2.4 and 3 ternary `a?[b].c():d`; a generic multi-declarator with a later value; a
// command call as an if or while condition; `in` as a local name; an empty index on null or a
// number; a quoted annotation member; a line break before an index or call inside parentheses
// or brackets; a multi-line slashy string at the start of a statement.
foo a*.b, [1]
foo q.@b, 1
foo a?.@b, 1
foo a.&b, 1
foo a::b, 1

def s = $/a//$
def f = a?[b].d():c
def fqs = params.fq?[params.fq].flatten().findAll { it != null }:null
config.stashContent = config.testRepository
    ?[GitUtils.handleTestRepository(this, config)]
    :utils.unstashAll(config.stashContent)

List<String> a, b = null
Map<String, Integer> m, n = [:]

if (a.b c[0]) {
    x()
}
while (a.b c) {
    x()
}

def g() {
    final InputStream in
    in = null
    return null[]
}

def y = 1[]

@interface R {
    String[] 'default'() default []
}

def t = f(a.x
    [0])
def r = f(1,
    Math.max
        (2, 3))
def l = [
    []
        [1],
]

def z = 1
/ *
 * text
/
