class Counter {
private int n = 0
void inc() {
n++
if (n > 10) {
reset()
}
}
void reset() {
n = 0
}
}
