enum Color {
    RED('r'),
    GREEN('g')

    final String code

    Color(String code) {
        this.code = code
    }
}

interface Shape {
    double area()
}

trait Named {
    String name
}
