// Constructs the twenty-fifth grammar batch added: a command call on a quoted name in an
// argument list or parentheses, and a name after its last argument there; a line-start slashy
// string with a backslash line continuation; type annotations on a wildcard and its bound, a
// throws type, a cast type, a catch parameter and array dimensions; `def final` closure
// parameters; a call whose parentheses are a lambda's parameters; type arguments after `*.`
// and `?.`.
/abc \
 x/

def m(l, e) {
    f("foo" 1)
    def r = ("foo" 1).bar
    f(a 1 b)
    f("a" 1 "b")
    def y = 1
    /abc \
 x/
    List<@A ? extends @B Object> w = null
    try { } catch (@A Exception x) { }
    def x = (@A String) e
    String @A [] s = null
    def a = new String @A [1] @B []
    def c = { def final int i, def final idx -> }
    foo.bar(String t) -> { t }
    def k = foo(z) -> z
    l*.<String>m()
    l?.<String>m(1)
}

void n() throws @A IOException, @B Exception {}
