// Constructs the twenty-second grammar batch added: a space before the type arguments of a
// closure's only parameter and of a for variable, with two or more of them; a generic type as
// a command call's argument (Groovy 3 and 4).
def xs = [[a: '1']]
xs.each { Map <String, String> e ->
    println e.a
}
def f = { Map <String, List<Integer>> e -> e.size() }
def g = { Map <String> e -> e }

for (Map <String, String> e : xs) {
    println e.a
}
for (Map <String, String> e in xs) {
    println e.a
}

def a = 1, b = 2, c = 3, d = 4
def h = [a < b, c > d]

// Groovy 3 and 4 only
def tag(Object... args) { args.toList() }

def m() {
    tag List<String>
    tag List<String>, 1
    tag 1, Map<String, List<Integer>>
    tag List<String>.class
    this.tag List<String>
}

def tags(Closure c) { c() }

def n() {
    tags {
        "input" List<String>
    }
    tags { "output" List<Map<String, int[]>> }
}
