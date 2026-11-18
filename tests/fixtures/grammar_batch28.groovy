// Constructs the twenty-eighth grammar batch added: a spread argument after a command call's
// first argument; an elvis as its first argument; a line break inside a declaration's type
// arguments; a typed closure parameter whose default is a call.
class B<X, Y> {}

def run(Object... a) { a.toList() }
def arguments = ['x', 'y']
def python = null
run 'gradlew', *arguments
run python, *arguments, '--info'
run python ?: 'python3', 'scripts/v.py'
run python ?: 'python3', *arguments

B<String,
        Integer> b = null
B<
        String, Integer> c = null

def d() { new File('x') }
def read = { File f = d() ->
    if (f) {
        return [a: 1]
    }
    [b: 2]
}
def other = { File f = d() -> if (f) { 1 } }
