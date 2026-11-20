// Constructs the thirty-second grammar batch added (#281): a generic record, also with a
// bounded type parameter, an interface and a compact constructor; and an anonymous class body
// whose initialized field, then a field initialized with a cast, then a method, stays a class
// body, beside the closure readings of a `{` after `new X()`.
record MapTransform<T>(Class<T> type, Closure<?> transformer) {}
record Pair<A, B extends Comparable<B>>(A left, B right) implements Serializable {
    Pair {
        assert left != null
    }
}
def o = new Object() {
    int r = 1
    String c = (String) 's'
    def f() { c }
}
def p = new Object() {
    int r = 1
    String c = (String) 's'
    int z = (int) 2L
    def f() { c + z }
}
def q = new Object() { -> 3 }
def a = new Object()
{ -> 3 }
