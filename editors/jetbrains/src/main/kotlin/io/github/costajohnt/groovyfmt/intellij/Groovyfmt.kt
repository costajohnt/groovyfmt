package io.github.costajohnt.groovyfmt.intellij

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.process.CapturingProcessHandler
import com.intellij.ide.impl.isTrusted
import com.intellij.ide.trustedProjects.TrustedProjectsListener
import com.intellij.notification.Notification
import com.intellij.notification.NotificationAction
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.Disposable
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.PathManager
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.service
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.progress.ProcessCanceledException
import com.intellij.openapi.options.ShowSettingsUtil
import com.intellij.openapi.progress.ProgressIndicator
import com.intellij.openapi.progress.ProgressManager
import com.intellij.openapi.progress.Task
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.psi.PsiFile
import com.intellij.util.EnvironmentUtil
import com.intellij.util.concurrency.SequentialTaskExecutor
import com.redhat.devtools.lsp4ij.LanguageServerEnablementSupport
import com.redhat.devtools.lsp4ij.LanguageServerFactory
import com.redhat.devtools.lsp4ij.LanguageServerManager
import com.redhat.devtools.lsp4ij.ServerStatus
import com.redhat.devtools.lsp4ij.client.features.LSPClientFeatures
import com.redhat.devtools.lsp4ij.client.features.LSPFormattingFeature
import com.redhat.devtools.lsp4ij.server.OSProcessStreamConnectionProvider
import com.redhat.devtools.lsp4ij.server.StreamConnectionProvider
import java.io.File
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.util.Properties
import java.util.concurrent.Future
import java.util.concurrent.atomic.AtomicBoolean

const val SERVER_ID = "groovyfmt"

/** Whether "groovyfmt was not found" was shown in this IDE session; Restart groovyfmt clears it. */
val notFoundShown = AtomicBoolean()
private val LOG = logger<GroovyfmtService>()

/** The files groovyfmt formats, as plugin.xml maps them to the server. */
private val FILE_NAMES = Regex("""^(.*\.(groovy|gvy|gy|gsh|gradle|jenkinsfile)|Jenkinsfile(\..*)?)$""")

fun isGroovyfmtFile(file: VirtualFile) = file.isInLocalFileSystem && FILE_NAMES.matches(file.name)

/** The project's directory, if it exists on disk: the working directory groovyfmt runs in. */
private fun projectDir(project: Project): String? = project.basePath?.takeIf { File(it).isDirectory }

/**
 * Which groovyfmt binary a project runs, found again on every [restart]. Nothing happens in a
 * project until [activate]: the first groovyfmt file opened in it, or a groovyfmt action. After
 * that it is found again when the settings change, when the project becomes trusted, and on
 * "Restart groovyfmt". The language server runs only while [binary] is set.
 */
@Service(Service.Level.PROJECT)
class GroovyfmtService(val project: Project) : Disposable {
    // finding the binary and starting the server run one at a time, in order
    private val queue = SequentialTaskExecutor.createSequentialApplicationPoolExecutor("Groovyfmt")

    @Volatile
    var binary: Found? = null
        private set

    /** Whether a groovyfmt file or action has needed the binary in this project yet. */
    @Volatile
    var active = false
        private set

    /** The last problem reported, for the tests. */
    @Volatile
    var lastProblem: String? = null
        private set

    /** The notification on show, replaced by the next one: one at a time. */
    @Volatile
    private var notice: Notification? = null

    /** LSP4IJ's own switch for the server, in its Language Servers view; on again at [restart]. */
    @Volatile
    var stoppedInLsp4ij = false

    override fun dispose() {}

    /** Finds the binary the first time a groovyfmt file or action needs it. */
    fun activate(): Future<*>? = if (active) null else restart()

    /** Finds the binary again, if anything in this project has needed it. */
    fun restartIfActive() {
        if (active) restart()
    }

    /**
     * Finds the binary again and restarts the server if it changed, or always with `force`
     * (Restart groovyfmt). Restarting a server that is still starting can leave its process
     * behind, so an unchanged binary leaves a running or starting server alone.
     */
    fun restart(force: Boolean = false): Future<*> = queue.submit {
        if (project.isDisposed) return@submit
        active = true
        stoppedInLsp4ij = false
        notice?.expire()
        val before = binary
        val found = resolve()
        binary = found
        ApplicationManager.getApplication().invokeLater({
            // A later restart may have changed the binary since: then this step is stale, and
            // starting the server could find the plugin turned off.
            if (binary !== found) return@invokeLater
            val manager = LanguageServerManager.getInstance(project)
            val running = manager.getServerStatus(SERVER_ID).let { it == ServerStatus.started || it == ServerStatus.starting }
            if (!force && found != null && found == before && running) return@invokeLater
            // a plain stop: LSP4IJ's default stop also turns the server off
            manager.stop(SERVER_ID, LanguageServerManager.StopOptions().setWillDisable(false))
            // starts it for the open files groovyfmt serves, if there are any
            if (found != null && !stoppedInLsp4ij) manager.start(SERVER_ID)
        }, project.disposed)
    }

    /**
     * Reports a problem in a notification. `missing`: groovyfmt simply isn't installed, which is
     * shown at most once per IDE session, and never after "Don't show again".
     */
    private fun problem(message: String, vararg actions: NotificationAction, missing: Boolean = false) {
        lastProblem = message
        LOG.warn(message)
        if (missing) {
            if (GroovyfmtSettings.get().silenceNotFound || !notFoundShown.compareAndSet(false, true)) return
        }
        val n = NotificationGroupManager.getInstance().getNotificationGroup("groovyfmt")
            .createNotification("groovyfmt", message, NotificationType.WARNING)
        actions.forEach { n.addAction(it) }
        if (missing) n.addAction(NotificationAction.createSimpleExpiring("Don't show again") { GroovyfmtSettings.get().silenceNotFound = true })
        n.addAction(NotificationAction.createSimple("Settings") {
            ShowSettingsUtil.getInstance().showSettingsDialog(project, GroovyfmtConfigurable::class.java)
        })
        notice?.expire()
        notice = n
        n.notify(project)
    }

    /** The binary to run, checked, or null after reporting why there is none. */
    private fun resolve(): Found? {
        lastProblem = null
        val settings = GroovyfmtSettings.get()
        if (!settings.enabled) {
            lastProblem = "groovyfmt is turned off in its settings."
            LOG.info(lastProblem)
            return null
        }
        val (platform, arch) = currentPlatform()
        val where = Where(
            appPath = settings.path,
            projectPath = GroovyfmtProjectSettings.get(project).path,
            trusted = project.isTrusted(),
            projectDir = project.basePath,
            envPath = EnvironmentUtil.getValue("PATH") ?: System.getenv("PATH").orEmpty(),
            platform = platform,
            arch = arch,
        )
        val found = when (val r = findBinary(where)) {
            is Problem -> return null.also { problem(r.message) }
            is Found -> r
            null -> downloaded(platform, arch) ?: return null.also { offerDownload(platform, arch) }
        }
        val version = try {
            parseVersion(runVersion(found.path))
        } catch (e: Exception) {
            problem("Could not run groovyfmt at ${found.path} (from ${found.source}): ${e.message}")
            return null
        }
        if (version == null) {
            problem("${found.path} (from ${found.source}) did not print a groovyfmt version.")
            return null
        }
        val v = version.joinToString(".")
        if (isOlder(version, MIN_LSP_VERSION.split('.').map { it.toInt() })) {
            problem("groovyfmt $v at ${found.path} (from ${found.source}) is too old: the language server (--lsp) arrived in $MIN_LSP_VERSION. Install a newer groovyfmt.")
            return null
        }
        LOG.info("Starting groovyfmt $v from ${found.source}: ${found.path}")
        return found
    }

    /** The plugin's version, which is groovyfmt's, written in at build time. */
    private fun pluginVersion(): String =
        GroovyfmtService::class.java.getResourceAsStream("/groovyfmt-version.txt")!!.use { it.readBytes().toString(Charsets.UTF_8).trim() }

    private fun downloadDest(platform: String, arch: String) =
        downloadPath(PathManager.getSystemDir().resolve("groovyfmt"), pluginVersion(), platform, arch)

    /** The binary this plugin downloaded earlier for its version. */
    private fun downloaded(platform: String, arch: String): Found? =
        downloadDest(platform, arch).takeIf { Files.isRegularFile(it) }?.let { Found(it.toString(), "download") }

    /** The checksums a release build carries; none in other builds. */
    private fun checksums(): Properties? =
        GroovyfmtService::class.java.getResourceAsStream("/groovyfmt-checksums.properties")?.use { Properties().apply { load(it) } }

    private fun offerDownload(platform: String, arch: String) {
        val v = pluginVersion()
        val missing = "groovyfmt was not found in the path settings, node_modules or PATH."
        if (TARGETS["$platform-$arch"] == null) {
            return problem("$missing There is no prebuilt binary for $platform-$arch; install groovyfmt and set its path.", missing = true)
        }
        val sha256 = embeddedSha256(checksums(), v, platform, arch)
            ?: return problem(
                "$missing This build of the plugin carries no checksums for groovyfmt $v (only release builds do), so it does not download groovyfmt. " +
                    "Install groovyfmt another way and set its path if it is not on PATH, or install a release build of the plugin.",
                missing = true,
            )
        if (GroovyfmtSettings.get().downloadDeclined) {
            return problem("$missing Install it, set its path, or run Tools, Restart groovyfmt to be offered the download again.", missing = true)
        }
        // dismissed without a choice, it is offered again after Restart groovyfmt or in the next IDE session
        problem(
            "$missing Download groovyfmt $v for $platform-$arch from its GitHub release? Before it runs, the download is checked against the SHA-256 checksum that this plugin release carries.",
            NotificationAction.createSimpleExpiring("Download") { download(v, platform, arch, sha256) },
            NotificationAction.createSimpleExpiring("Not now") { GroovyfmtSettings.get().downloadDeclined = true },
            missing = true,
        )
    }

    private fun download(v: String, platform: String, arch: String, sha256: String) {
        val dest = downloadDest(platform, arch)
        ProgressManager.getInstance().run(object : Task.Backgroundable(project, "Downloading groovyfmt $v", true) {
            override fun run(indicator: ProgressIndicator) = download(v, platform, arch, dest, sha256)
            override fun onSuccess() {
                LOG.info("Downloaded groovyfmt $v to $dest, SHA-256 matches the checksum this plugin carries")
                restart()
            }
            override fun onThrowable(error: Throwable) = problem("Downloading groovyfmt failed: ${error.message}")
        })
    }
}

/**
 * Finds the binary when the first groovyfmt file opens in a project (a project without Groovy
 * files never runs or reports anything), and again when the project becomes trusted.
 */
class GroovyfmtStartup : ProjectActivity {
    override suspend fun execute(project: Project) {
        val service = project.service<GroovyfmtService>()
        ApplicationManager.getApplication().messageBus.connect(service)
            .subscribe(TrustedProjectsListener.TOPIC, object : TrustedProjectsListener {
                override fun onProjectTrusted(project: Project) {
                    if (project == service.project) service.restartIfActive()
                }
            })
        project.messageBus.connect(service).subscribe(FileEditorManagerListener.FILE_EDITOR_MANAGER, object : FileEditorManagerListener {
            override fun fileOpened(source: FileEditorManager, file: VirtualFile) {
                if (isGroovyfmtFile(file)) service.activate()
            }
        })
        if (FileEditorManager.getInstance(project).openFiles.any(::isGroovyfmtFile)) service.activate()
    }
}

/** The language server, for LSP4IJ: runs the service's binary with `--lsp`. */
class GroovyfmtServerFactory : LanguageServerFactory, LanguageServerEnablementSupport {
    override fun createConnectionProvider(project: Project): StreamConnectionProvider {
        val found = project.service<GroovyfmtService>().binary
            // LSP4IJ starts the server only while isEnabled, so there is a binary
            ?: throw IllegalStateException("groovyfmt: no binary to run")
        return OSProcessStreamConnectionProvider(
            GeneralCommandLine(found.path, "--lsp").withWorkDirectory(projectDir(project)).withCharset(StandardCharsets.UTF_8),
        )
    }

    override fun createClientFeatures(): LSPClientFeatures = LSPClientFeatures().setFormattingFeature(object : LSPFormattingFeature() {
        // Off by default: in a Groovy file Reformat Code stays the IDE's own formatter. A file
        // the IDE has no formatter for is formatted by groovyfmt either way.
        override fun isExistingFormatterOverrideable(file: PsiFile) = GroovyfmtSettings.get().useForReformatCode
    })

    override fun isEnabled(project: Project): Boolean {
        val service = project.service<GroovyfmtService>()
        return service.binary != null && !service.stoppedInLsp4ij
    }

    // LSP4IJ turns a server off from its Language Servers view, and after it fails to start
    // several times. LSP4IJ's restart turns it on again, as does Restart groovyfmt.
    override fun setEnabled(enabled: Boolean, project: Project) {
        project.service<GroovyfmtService>().stoppedInLsp4ij = !enabled
    }
}

/**
 * Code, Reformat with groovyfmt: the whole file through `groovyfmt --stdin`, whatever formats
 * Reformat Code. Runs the binary the language server runs.
 */
class GroovyfmtReformatAction : AnAction() {
    override fun getActionUpdateThread() = ActionUpdateThread.BGT

    override fun update(e: AnActionEvent) {
        val file = e.getData(CommonDataKeys.VIRTUAL_FILE)
        e.presentation.isEnabledAndVisible = e.project != null && e.getData(CommonDataKeys.EDITOR) != null && file != null && isGroovyfmtFile(file)
    }

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return
        val file = e.getData(CommonDataKeys.VIRTUAL_FILE) ?: return
        reformat(project, editor.document, file)
    }

    companion object {
        /** Formats `document` (the contents of `file`) with groovyfmt, in the background. */
        fun reformat(project: Project, document: com.intellij.openapi.editor.Document, file: VirtualFile) {
            val service = project.service<GroovyfmtService>()
            val bin = service.binary ?: run {
                // the first groovyfmt action in a project finds the binary; after that, a
                // missing binary has been reported, and this asks again
                service.restart()
                return
            }
            val text = document.text
            val stamp = document.modificationStamp
            ProgressManager.getInstance().run(object : Task.Backgroundable(project, "Reformatting with groovyfmt", true) {
                var out: String? = null
                override fun run(indicator: ProgressIndicator) {
                    out = format(bin.path, text, file.path, projectDir(project), indicator)
                }
                override fun onSuccess() {
                    val formatted = out ?: return
                    if (formatted == text || document.modificationStamp != stamp) return
                    WriteCommandAction.runWriteCommandAction(project, "Reformat with groovyfmt", null, { document.setText(formatted) })
                }
                override fun onThrowable(error: Throwable) {
                    NotificationGroupManager.getInstance().getNotificationGroup("groovyfmt")
                        .createNotification("groovyfmt", error.message ?: error.toString(), NotificationType.WARNING).notify(project)
                }
            })
        }

        /**
         * `text` formatted as the file at `path`; throws with groovyfmt's message if it can't be.
         * The process is killed past `timeoutMs` or when `indicator` is cancelled.
         */
        fun format(bin: String, text: String, path: String, cwd: String?, indicator: ProgressIndicator? = null, timeoutMs: Int = 60_000): String {
            val cmd = GeneralCommandLine(bin, "--stdin", "--stdin-filepath", path).withCharset(StandardCharsets.UTF_8)
            if (cwd != null) cmd.withWorkDirectory(cwd)
            val handler = CapturingProcessHandler(cmd)
            // Written from another thread while the output is read and the timeout runs: a
            // document larger than the pipe would otherwise block here, before either starts,
            // for as long as the process doesn't read it. Killing the process ends the write.
            ApplicationManager.getApplication().executeOnPooledThread {
                try {
                    handler.processInput.use { it.write(text.toByteArray(StandardCharsets.UTF_8)) }
                } catch (e: java.io.IOException) {
                    // the process exited or was killed without reading all of it
                }
            }
            val r = if (indicator != null) handler.runProcessWithProgressIndicator(indicator, timeoutMs) else handler.runProcess(timeoutMs)
            if (r.isCancelled) throw ProcessCanceledException()
            if (r.isTimeout) throw java.io.IOException("groovyfmt did not finish within ${timeoutMs / 1000} seconds")
            if (r.exitCode != 0) throw java.io.IOException("groovyfmt could not format ${File(path).name}: ${r.stderr.trim()}")
            // the document holds \n line endings whatever the file has
            return r.stdout.replace("\r\n", "\n")
        }
    }
}

/** Tools, Restart groovyfmt: finds the binary again, and may offer the download again. */
class GroovyfmtRestartAction : AnAction() {
    override fun getActionUpdateThread() = ActionUpdateThread.BGT

    override fun update(e: AnActionEvent) {
        e.presentation.isEnabled = e.project != null
    }

    override fun actionPerformed(e: AnActionEvent) {
        GroovyfmtSettings.get().downloadDeclined = false
        notFoundShown.set(false)
        e.project?.service<GroovyfmtService>()?.restart(force = true)
    }
}
