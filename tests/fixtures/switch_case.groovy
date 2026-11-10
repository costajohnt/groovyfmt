String describe(Object x) {
switch (x) {
case Integer:
return 'int'
case ['a', 'b']:
def s = x.toString()
return s * 2
case ~/z+/:
return 'zs'
default:
return 'other'
}
}
