// `groovyfmt: off` regions: nested in a block, inside a closure, and one left open to the
// end of the file. Their lines stay exactly as written.
class Off {
    def f() {
        // groovyfmt: off
        int[][] m = [[1,0],
                     [0,1]]
           def   weird  =  1
        // groovyfmt: on
        println m
    }

    def g() {
        [1, 2].each { x ->
            /* groovyfmt: off */
          println   x
                if (x) { x }
            /* groovyfmt: on */
            println x
        }
    }
}

def tail() {
    1
}
// groovyfmt: off
def   unterminated() {
  2
      }
