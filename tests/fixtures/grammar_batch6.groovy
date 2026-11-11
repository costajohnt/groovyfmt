// Constructs the sixth grammar batch added.
class Batch6Controller {
    def stats(a, b, x, y, line) {
        println "${a/1000f},${b/1000f}"
        log.info("[${x.size()/1024}] KB, at most ${y / 1024} KB")
        def ratio = a/2 + b/2
        def parts = line.split(/=/, 2)
        render [:] as JSON
    }

    void save() {
        def plugins = [
            new Plugin(name: 'a'),
            new Plugin(name: 'b')
        ]*.save validate:false, flush:true
        localeResolver?.setLocale request, response, locale
    }

    def ui(final @DelegatesTo(strategy = Closure.DELEGATE_FIRST, value = BlockSpec) Closure closure) {
        form(new UiFormSpecifier().ui new Attachment(), {
            section {
                field a.active_
            }
        })
        menuIcon ext == 'pdf' ? ActionIcon.EXPORT_PDF : ActionIcon.EXPORT, Controller.&show as MC, [id: 1]
    }
}
