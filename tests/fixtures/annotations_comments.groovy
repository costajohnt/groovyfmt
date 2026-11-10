import groovy.transform.CompileStatic

@CompileStatic // fast path
class Service {
    @Deprecated // use fetchAll
    List fetch() { [] }

    @SuppressWarnings('unused') /* kept for the plugin API */
    private String legacy

    @Override
    String toString() { 'Service' }
}
