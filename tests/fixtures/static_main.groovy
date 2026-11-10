class App {
    static final int LIMIT = 10
    private static final Map<String, Integer> CACHE = [:]

    static void main(String[] args) {
        args.eachWithIndex { a, i ->
            if (i < LIMIT) println "${i}: ${a}"
        }
    }
}
