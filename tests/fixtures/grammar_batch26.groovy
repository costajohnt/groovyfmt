// Constructs the twenty-sixth grammar batch added: a braceless if body that is a command call
// whose one argument is a call, before `else`; a command call as a declaration's or
// assignment's value continued with a backslash onto named arguments on the next lines.
if (b)
    p.accept f(t())
else
    g()

if (b) accept f([1]) else g()

if (h(c, "("))
    g()
// Read field
else if (ctx.fieldSig())
    processor.accept fieldUsage(ctx.fieldSig(), UsageKind.DATA_READ)
    /* Phi */
else
    h()

def m() {
    if (a) p.accept f(c, U.D)
    else h()
    def r = sh \
        script: "x",
        returnStatus: true
    r = sh \
        script: "y"
    r
}
