// Constructs the ninth grammar batch added: declarations and loops in a GString `${ }`
// body, annotations after modifiers, a comment after a `\` line continuation, comma lists
// in a classic `for` (Groovy 3 and 4), `in` as a variable name (Groovy 3 and 4) and an
// empty subscript on a string, next to the forms around them that stay as they were.
import groovy.transform.Field

static @groovy.transform.Field x = 1
private static @Field String y = "GET"
@Field static z = 2

class Batch9 {
    static @Deprecated x = 1
    static @Deprecated f() {}
    private @Deprecated String g() { null }
    static w = 3

    def strings(List xs, boolean c) {
        def a = "${ def v = 1; v }"
        def b = "${ for (i in xs) {} }"
        def d = "${ while (c) { c = false } }"
        def e = "${ try { xs[0] } finally { c = true } }"
        def f = "${ String r = xs[0]; r }"
        def g = "${ for (int i = 0; i < 2; i++) { r += i } }"
        def k = "${ if (c) { 1 } }"
        def l = "${xs.size()} and ${ xs.join ', ' }"
        [a, b, d, e, f, g, k, l]
    }

    def continued(a, b) {
        final @Deprecated q = 1
        def r = a || \
            // c
            b
        def s = a || \
            /* c */ b
        def t = a && \
            b
        r && s && t
    }

    def subscripts() {
        def y = "x"[]
        def z = "x"[0]
        def l = [][]
        [y, z, l]
    }

    def loops(List l) {
        for (int i = 0; i < 3; i++) {}
        for (x in l) {}
        for (String s in l) {}
        for (;;) { break }
        boolean b = 1 in l
        b
    }
}

def command(Object o) { o }
command "x" []

// Groovy 3 and 4 only from here (`do` and `!in` are too).
class Batch9Groovy3 {
    def loops(int n) {
        int j = 0
        for (int i = 0; i < n; i++, j++) {}
        for (int i = 0, k = 0; i < n; i++,k++) {}
        for (i = 0, j = 0; i < n; i++) {}
        InputStreamReader in = null
        in = new InputStreamReader(System.in)
        in.close()
        boolean c = 1 !in [j]
        "${ do { c = false } while (c) }"
    }
}
