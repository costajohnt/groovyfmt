// a backslash ends a line inside a quoted string: the string goes on (#235)
def s = 'a,\
b'
def g = "x ${s}, \
y"
class C {
    def f() {
        println('one \
two')
    }
}
println "$s \
$g"
