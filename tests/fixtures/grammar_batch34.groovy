// Constructs the thirty-fourth grammar batch added (#287): closure parameters whose default is a
// call with arguments, which Groovy 2.4, 3 and 4 read as the parameter's default.
def a = { y = f(1, 2) -> y }
def b = { x, y = helper(1, 2) -> x }
def c = { y = f(a) -> y }
def d = { Integer y = o.f(1, 'b') ->
    y
}
