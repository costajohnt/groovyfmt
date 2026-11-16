// Constructs the twentieth grammar batch added: an array creation with two or more trailing
// empty dimensions; `!in` right before `[` or `(` (Groovy 3 and 4); attribute access with the
// `@` on the next line; a record compact constructor (Groovy 4); a command call with a
// parenthesized receiver; a command chain inside parentheses (Groovy 3 and 4); a closure on the
// line after `new X()` where a statement may end, and one with `->` on its line; a closure
// parameter default that ends in a name.
def i = new int[10][][]
int[][][] j = new int[2][3][]

def n = 1 !in[2, 3]
def m = 1 !in(n)

def l = [new Object()]
def x = l*.
    @class

record Fruit(String name) {
    Fruit {
        assert name
    }
}

((int) 1 / 2) 1, 2
def p = (1.plus 2 plus 3)

def a = new Object()
    { int y = 1 }
def b = new Object()
    { }
def c = new Object() { -> 3 }

def d = { u, e = x + i -> e }
