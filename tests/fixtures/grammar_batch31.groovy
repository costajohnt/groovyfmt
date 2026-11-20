// Constructs the thirty-first grammar batch added (#261): a parenthesized command call whose
// argument is an assignment, an unspaced ternary or elvis, or a safe index with an operator,
// or has a comment, a line break or a backslash continuation before its operator; and a
// lambda or script method whose parameter has a default value, which stays one.
x = (a b = c)
if (a.b c?d:e) {
    y()
}
x = (a b?:c)
x = (a b /* c */ - d)
x = (a b
    - c)
x = (a b \
    - c)
x = (a b // c
    - d)
x = (a l?[0] - c)
f = (String s = 'x') -> s
List<String> m(Map p = [:]) {
    p.keySet() as List
}
