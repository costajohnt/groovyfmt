// Constructs the twenty-seventh grammar batch added: a declaration with modifiers and no
// type at a script's top level or in a block; an argument list on the line after a list
// element; a typed closure parameter named `in`.
private static final _JOB_DISPLAY_NAME = 'Deploy'
private static final _JOB_DESCRIPTION = '''\
Deploys the build.
'''
static final RETRIES = 3, DELAY = 10
private count = 0
public static NAME = "x"

def m() {
    static final LIMIT = 5
    private seen = [:]
    if (count < LIMIT) {
        protected final n = count + 1
        seen[n] = true
    }
    seen
}

properties(
    [
    buildDiscarder
        (logRotator (
            daysToKeepStr: '14',
            numToKeepStr: '10'
        ) ),
    disableConcurrentBuilds()
    ]
)
def l = [ a.b
    (1), f
    (2)
    (3), g(4) ]

stream.withCloseable { InputStream in ->
    Files.copy(in, target)
}
def c = { final java.io.InputStream in, int n -> in.read() }
