// Constructs the twenty-third grammar batch added: a braceless if body that is a command call
// with a name argument, then `else` on its line; a command call's argument continued by `?` or
// `?:` on the next line.
def a = true, b = false
def line = 'x'
if (a) warn line else info line
if (a) log.warn line else log.info line
if (a) warn line else if (b) info line else debug line
if (a) foo bar baz qux else z

def m(x) {
    if (a) log.warn line else log.info line
    x.url isSnapshot
        ? "a"
        : "b"
    x.url isSnapshot
        ?: 2
    x?.url isSnapshot
        ? 1 : 2
    foo x
        ? 1
        : 2
}

def f = { url isSnapshot
    ? 1
    : 2 }
