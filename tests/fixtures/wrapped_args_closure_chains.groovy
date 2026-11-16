// #183: a trailing closure after wrapped arguments, and a chain on the line after `=`
f.entry(title: 'Override Hook URL') {
    g.blockWrapper {
        f.optionalBlock(title: 'Specify another hook URL',
                name: 'isOverrideHookUrl',
                inline: true,
                checked: instance.isOverrideHookUrl()) {
            f.entry(field: 'hookUrl') {
                f.textbox(checkMethod: 'post', name: 'hookUrl')
            }
        }
    }
}

tasks.register('openApi', Exec,
    'a', 'b') {
    commandLine 'x'
}

def canonicalise(Map schema) {
    Map normalised = [:]
    schema.keySet().sort().each { k ->
        Object v = schema[k]
        normalised[k] =
            (v instanceof Map) ? canonicalise(v) :
            (v instanceof List) ? v.collect {
                it instanceof Map ? canonicalise(it) : it
            } :
            v
    }
    def label =
        user?.name ?:
        user?.login ?:
        'anonymous'
    foo(
        a ?: b ?:
        c,
        d)
    normalised
}
