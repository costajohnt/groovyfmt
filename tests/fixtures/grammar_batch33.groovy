// Constructs the thirty-third grammar batch added (#283): closures whose first parameter has a
// generic or array type or a default, followed by more parameters, beside a lambda with the same
// parameters and a generic declaration of two names in a closure.
def a = { Map<String, List<Integer>> m, int n -> (String) m }
def b = { List<?> m, Map<String, ? extends Number> o, int n -> m }
def c = { String[] m, int n -> m }
def d = { Map<String, Integer> m = [:], int n -> m }
def e = { @Deprecated Map<String, Integer> m, final int n -> m }
def f = { java.util.Map<String, Integer> m, int... n -> m }
def g = {
    Map<String, Integer> m,
        int n ->
    m
}
def h = { (Map<String, Integer> m, int n) -> m }
def i = { Map<String, Integer> m, n
    m }
