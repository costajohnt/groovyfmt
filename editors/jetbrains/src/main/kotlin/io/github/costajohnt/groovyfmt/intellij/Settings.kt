package io.github.costajohnt.groovyfmt.intellij

import com.intellij.openapi.components.BaseState
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.SimplePersistentStateComponent
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage
import com.intellij.openapi.components.service
import com.intellij.openapi.options.BoundConfigurable
import com.intellij.openapi.project.Project
import com.intellij.openapi.project.ProjectManager
import com.intellij.ui.dsl.builder.bindSelected
import com.intellij.ui.dsl.builder.bindText
import com.intellij.ui.dsl.builder.columns
import com.intellij.ui.dsl.builder.panel

/** The IDE-wide settings. */
@Service(Service.Level.APP)
@State(name = "groovyfmt", storages = [Storage("groovyfmt.xml")])
class GroovyfmtSettings : SimplePersistentStateComponent<GroovyfmtSettings.Options>(Options()) {
    class Options : BaseState() {
        var enabled by property(true)
        /** Absolute path to the binary. */
        var path by string()
        /** Reformat Code uses groovyfmt in files the IDE has its own formatter for. */
        var useForReformatCode by property(false)
        var downloadDeclined by property(false)
        /** "groovyfmt was not found" is never shown ("Don't show again"). */
        var silenceNotFound by property(false)
    }

    companion object {
        fun get(): Options = service<GroovyfmtSettings>().state
    }
}

/**
 * The project's settings, stored in .idea/groovyfmt.xml. They come with the project, so they
 * count only in a trusted project.
 */
@Service(Service.Level.PROJECT)
@State(name = "groovyfmt", storages = [Storage("groovyfmt.xml")])
class GroovyfmtProjectSettings : SimplePersistentStateComponent<GroovyfmtProjectSettings.Options>(Options()) {
    class Options : BaseState() {
        /** Path to the binary, absolute or relative to the project directory. */
        var path by string()
    }

    companion object {
        fun get(project: Project): Options = project.service<GroovyfmtProjectSettings>().state
    }
}

/** Settings, Tools, groovyfmt. */
class GroovyfmtConfigurable(private val project: Project) : BoundConfigurable("groovyfmt") {
    override fun createPanel() = panel {
        val app = GroovyfmtSettings.get()
        val local = GroovyfmtProjectSettings.get(project)
        row { checkBox("Run the groovyfmt language server").bindSelected(app::enabled) }
        row("Path to groovyfmt:") {
            textField().columns(40).bindText({ app.path.orEmpty() }, { app.path = it.trim().ifEmpty { null } })
                .comment("For all projects. An absolute path. Empty: the project's setting, node_modules, then PATH, then a download.")
        }
        row("Path for this project:") {
            textField().columns(40).bindText({ local.path.orEmpty() }, { local.path = it.trim().ifEmpty { null } })
                .comment("Stored in .idea/groovyfmt.xml, absolute or relative to the project directory. Used over the path above, and only in a trusted project.")
        }
        row {
            checkBox("Use groovyfmt for Reformat Code").bindSelected(app::useForReformatCode)
                .comment("Off: Reformat Code keeps the IDE's own Groovy formatter, and Code, Reformat with groovyfmt runs groovyfmt. On: Reformat Code, and so the Reformat code action on save, runs groovyfmt.")
        }
        row {
            checkBox("Notify when groovyfmt is not installed")
                .bindSelected({ !app.silenceNotFound }, { app.silenceNotFound = !it })
                .comment("At most once per IDE session, in a project with Groovy, Gradle or Jenkins files open.")
        }
    }

    override fun apply() {
        super.apply()
        // the IDE-wide settings change every open project's binary
        for (p in ProjectManager.getInstance().openProjects) p.service<GroovyfmtService>().restartIfActive()
    }
}
