def load(String path) {
    def reader = null
    try {
        reader = new File(path).newReader()
        return reader.readLines()
    } catch (FileNotFoundException | SecurityException e) {
        log.warn("cannot read ${path}", e)
        return []
    } finally {
        reader?.close()
    }
}
