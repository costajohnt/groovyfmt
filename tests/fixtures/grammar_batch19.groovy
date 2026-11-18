// Constructs the nineteenth grammar batch added: a closure on the line after `new X()`, which
// Groovy passes to the new object (in parentheses a class body there stays one); a command call
// as a call's first argument (Groovy 3 and 4); a backslash continuation after a named argument's
// comma; a block comment in type arguments; an interface method with no return type (Groovy 3
// and 4); `return` inside `${}`.
def a = new Object()
{ -> 3 }
def r = new Runnable() {
    void run() { }
}
def t = new Thread(new Runnable()
{
    void run() { }
})

println(g 1)
def json = parseText(readJSON file: "x")

f a: 1, \
    b: 2

Map<String, Collection<? /*V*/>> m = [:]
List<? extends /*V*/ Number> l = []

interface K {
    key()
    value(String s) throws IOException
}

def x = 5
def s1 = "${-> println 1; return x}"
def s2 = "${return x}"
