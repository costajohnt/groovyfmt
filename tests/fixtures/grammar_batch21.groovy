// Constructs the twenty-first grammar batch added: `sealed` as a name; a braceless if body
// that is a command call with a cast argument, then `else`; a number as the key of a command
// call's named argument; a closure parameter default whose right operand nests and ends in a
// name; `as` and `in` as names, also a typed closure parameter `as` and `println as[0]` (Groovy 3 and 4).
def sealed = 1
println sealed

def render(x) { x }
def result = [a: 1]
if (sealed)
    render result as String
else
    render status: 404

def select(m) { m }
select 1:-1
select 1.5: 2, 3: 4

def y = 2, g = 3
def f = { a, e = sealed + y * g -> e }

// Groovy 3 and 4 only
def m() {
    String as = 'x'
    as.size()
    as
}

def n(List<String> xs) {
    List<String> as = xs
    def p = (Object) as[0]
}

def o(List<String> xs) {
    for (String as : xs) {
        println as
    }
}

def k() {
    String in
    if (sealed) {
        in = 'a'
    }
    println in
    in
}

def q(List<String> xs) {
    Closure<Boolean> has = { List<String> as, String n -> as.any { it == n } }
    def r = { String as -> as }
    def s = { final String as = 'a', b -> as + b }
    List<String> as = xs
    println as[0]
    has(as, 'x')
}
