// Constructs the thirty-first grammar batch added (#261): a parenthesized command call whose
// argument is an assignment, an unspaced ternary or elvis, or a safe index with an operator,
// or has a comment, a line break or a backslash continuation before its operator; and a
// lambda or script method whose parameter has a default value, which stays one, also with a
// comment or a backslash continuation before its `{`, `throws` or `->`.
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
List<String> m1(Map p = [:]) // c
{
    p
}
List<String> m2(Map p = [:]) /* c */ {
    p
}
List<String> m3(Map p = [:])
// c
{
    p
}
List<String> m4(Map p = [:]) /* c */ throws IOException {
    p
}
List<String> m5(Map p = [:]) \
{
    p
}
String[] m6(String s = 'x') /* c */ {
    s
}
Map<String, Integer> m7(Map p = [:]) // c
{
    p
}
List<String> m8(Map p = [:]) // c
    throws IOException {
    p
}
def f1 = (String s = 'x') /* c */ -> s
def f2 = (String s = 'x') // c
    -> s
f3 = (String s = 'x', int n = 2) /* two */ -> s * n
x = (a b = 1, c)
if ((a b = c)) {
}
