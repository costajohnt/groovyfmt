class Wrapper {
    def build(String name,
            int size,
            Map options = [:]) {
        if (name &&
                size > 0 &&
                options.enabled) {
            return make(name,
                    size)
        }
        while (size > 0
                && name) {
            size--
        }
    }
}
