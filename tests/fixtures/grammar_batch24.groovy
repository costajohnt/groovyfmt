// Constructs the twenty-fourth grammar batch added: a command call in parentheses on a call, or
// with a name after its last argument; a slashy string starting with a space at the start of a
// line; a method reference to `class`; modifiers and annotations after `sealed`.
def m(a, b) {
    def r = (spelling(a) intersect b).isEmpty()
    def s = (x.f(a) intersect g(b))
    if (f(a) intersect b) println r
    def t = (a intersect b)
    foo(x, A::class)
    def k = A::class.name
}

def rows() {
    a | b
    / 'test1' 'test2' / | 'test1'
    / ('test1'|'test2') / | ''
    def y = 1
    / x /.size()
}

sealed abstract class W { }
sealed @Deprecated class U { }
non-sealed abstract class T { }
