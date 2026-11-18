// Constructs the twenty-ninth grammar batch added: a command call on a call as an argument
// list's first argument; a braceless if body's command call on an operator argument before
// `else`; a parenthesized first argument of a command call inside parentheses.
def shadow(Object... a) { a.toList() }
def implementation(Object... a) { a.toList() }
def project(String p) { p }
shadow(implementation project(':lib'))
shadow(implementation project(':lib'), 'x')

class O {
    def f(x) { x }
}
def o = new O()
def a = 5
def b = 2
for (c in [true, false]) {
    if (c) o.f a - b
    else o.f 0
    if (c) o.f a * b + 1 // more
    else o.f 0
}

def sep = '3B'
def ch = (Integer.parseInt(sep), 16) as char
def n = (Integer.parseInt(sep), 16)
