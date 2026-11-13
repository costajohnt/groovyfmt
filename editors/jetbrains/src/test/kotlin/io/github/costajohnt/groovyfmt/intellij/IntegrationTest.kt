package io.github.costajohnt.groovyfmt.intellij

import com.intellij.codeInsight.daemon.impl.HighlightInfo
import com.intellij.ide.impl.isTrusted
import com.intellij.ide.impl.setTrusted
import com.intellij.lang.annotation.HighlightSeverity
import com.intellij.openapi.actionSystem.IdeActions
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.components.service
import com.intellij.openapi.editor.impl.DocumentMarkupModel
import com.intellij.testFramework.PlatformTestUtil
import com.intellij.testFramework.UsefulTestCase
import com.intellij.testFramework.fixtures.CodeInsightTestFixture
import com.intellij.testFramework.fixtures.IdeaTestFixtureFactory
import com.intellij.testFramework.fixtures.impl.TempDirTestFixtureImpl
import com.redhat.devtools.lsp4ij.LanguageServerManager
import com.redhat.devtools.lsp4ij.LanguageServersRegistry
import com.redhat.devtools.lsp4ij.ServerStatus
import com.redhat.devtools.lsp4ij.server.definition.launching.UserDefinedLanguageServerDefinition
import com.redhat.devtools.lsp4ij.templates.ServerMappingSettings
import java.io.File
import java.nio.file.Files
import java.util.concurrent.TimeUnit

// A lint violation and indentation groovyfmt fixes. groovyfmt keeps at most one blank line in a
// row and the IDE's Groovy formatter keeps these three, so the result shows which formatter ran.
private const val SAMPLE = "class A {\ndef f() {\nprintln 1;\n\n\n\nprintln 2\n}\n}\n"
private const val FORMATTED = "class A {\n    def f() {\n        println 1;\n\n        println 2\n    }\n}\n"
// the IDE's Groovy formatter indents and keeps all three blank lines
private const val IDE_FORMATTED = "class A {\n    def f() {\n        println 1;\n\n\n\n        println 2\n    }\n}\n"

/**
 * The plugin in the IDE with LSP4IJ, on files on disk (groovyfmt serves only file: documents).
 * Needs a groovyfmt build, named by the GROOVYFMT_BIN environment variable.
 */
class IntegrationTest : UsefulTestCase() {
    private lateinit var fixture: CodeInsightTestFixture

    override fun setUp() {
        super.setUp()
        val factory = IdeaTestFixtureFactory.getFixtureFactory()
        fixture = factory.createCodeInsightFixture(factory.createFixtureBuilder(name).fixture, TempDirTestFixtureImpl())
        fixture.setUp()
    }

    override fun tearDown() {
        try {
            GroovyfmtSettings.get().apply {
                enabled = true
                path = null
                useForReformatCode = false
            }
            // the server's process and its reader threads end before the leak check runs
            val manager = LanguageServerManager.getInstance(fixture.project)
            manager.stop(SERVER_ID, LanguageServerManager.StopOptions().setWillDisable(false))
            val end = System.currentTimeMillis() + 30_000
            // a server process still running: the threads reading its output
            fun serverThreads() = Thread.getAllStackTraces().keys.count { it.isAlive && it.name.startsWith("BaseDataReader") && it.name.contains(" --lsp") }
            while ((manager.getServerStatus(SERVER_ID).let { it != ServerStatus.stopped && it != ServerStatus.none } || serverThreads() > 0) &&
                System.currentTimeMillis() < end
            ) {
                PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
                Thread.sleep(100)
            }
            fixture.tearDown()
        } catch (e: Throwable) {
            addSuppressedException(e)
        } finally {
            super.tearDown()
        }
    }

    private fun bin(): String {
        val bin = System.getProperty("groovyfmt.bin")
        assertTrue("set GROOVYFMT_BIN to the absolute path of a groovyfmt binary", bin != null && File(bin).isAbsolute)
        expectServerThreads(bin)
        return bin
    }

    /**
     * LSP4IJ runs each server's messages on a cached thread pool named after its command line;
     * an idle thread there outlives the stopped server by up to a minute. Those are not the
     * plugin's leak, so the leak check skips threads named after this binary.
     */
    private fun expectServerThreads(binary: String) {
        com.intellij.testFramework.common.ThreadLeakTracker.longRunningThreadCreated(testRootDisposable, "$binary --lsp")
    }

    /** Runs the IDE's event queue until `done` or the timeout. */
    private fun waitFor(what: String, timeoutMs: Long, done: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMs
        while (!done()) {
            if (System.currentTimeMillis() > end) fail("timed out after $timeoutMs ms waiting for $what")
            PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
            Thread.sleep(100)
        }
    }

    private fun restart() {
        fixture.project.service<GroovyfmtService>().restart().get(30, TimeUnit.SECONDS)
        PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
    }

    private fun reset() {
        WriteCommandAction.runWriteCommandAction(fixture.project) { fixture.editor.document.setText(SAMPLE) }
    }

    /**
     * Opens the sample. The IDE highlights an opened file at once, which sets up LSP4IJ's
     * diagnostics; a test has to ask. It asks on a file groovyfmt doesn't handle: opening the
     * sample starts the plugin, whose server start would land in the middle of highlighting it.
     */
    private fun open() {
        fixture.configureByText("warmup.txt", "")
        fixture.doHighlighting()
        val file = fixture.configureByText("A.groovy", SAMPLE)
        assertTrue(file.virtualFile.isInLocalFileSystem)
    }

    /** LSP4IJ puts diagnostics straight into the document's markup, outside the highlighting passes. */
    private fun waitForSemicolonDiagnostic(): HighlightInfo {
        var semicolon: HighlightInfo? = null
        waitFor("the UnnecessarySemicolon diagnostic", 60_000) {
            semicolon = DocumentMarkupModel.forDocument(fixture.editor.document, fixture.project, true).allHighlighters
                .mapNotNull { HighlightInfo.fromRangeHighlighter(it) }
                .firstOrNull { it.description?.contains("Semicolons as line endings") == true }
            semicolon != null
        }
        return semicolon!!
    }

    fun testDiagnosticsAndFormatting() {
        val bin = bin()
        GroovyfmtSettings.get().path = bin
        open()
        restart()
        assertEquals(bin, fixture.project.service<GroovyfmtService>().binary?.path)

        // the server starts for the open file, and a lint diagnostic arrives
        waitFor("the server to start", 30_000) {
            LanguageServerManager.getInstance(fixture.project).getServerStatus(SERVER_ID) == ServerStatus.started
        }
        repeat(10) {
            PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
            Thread.sleep(100)
        }
        // the IDE highlights an open editor by itself, which connects the file to the server; a
        // test has to ask (and only now, with the server's start behind it)
        fixture.doHighlighting()
        val semicolon = waitForSemicolonDiagnostic()
        assertEquals(HighlightSeverity.WARNING, semicolon.severity)
        assertEquals(2, fixture.editor.document.getLineNumber(semicolon.startOffset))

        // Reformat Code keeps the IDE's own formatter by default: its output, and still its
        // output after as long as groovyfmt is given below
        fixture.performEditorAction(IdeActions.ACTION_EDITOR_REFORMAT)
        PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
        assertEquals(IDE_FORMATTED, fixture.editor.document.text)
        val end = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < end) {
            PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
            Thread.sleep(100)
        }
        assertEquals(IDE_FORMATTED, fixture.editor.document.text)

        // Reformat with groovyfmt
        reset()
        fixture.performEditorAction("groovyfmt.Reformat")
        waitFor("Reformat with groovyfmt", 30_000) { fixture.editor.document.text == FORMATTED }

        // Reformat Code with groovyfmt, once the setting says so
        reset()
        GroovyfmtSettings.get().useForReformatCode = true
        fixture.performEditorAction(IdeActions.ACTION_EDITOR_REFORMAT)
        waitFor("Reformat Code to use groovyfmt", 30_000) { fixture.editor.document.text == FORMATTED }

        // the server's fix-all code action
        val fixAll = fixture.availableIntentions.firstOrNull { it.text == "Fix all auto-fixable problems" }
        assertNotNull("no fix-all action in ${fixture.availableIntentions.map { it.text }}", fixAll)
        fixture.launchAction(fixAll!!)
        waitFor("fix all", 30_000) { fixture.editor.document.text == FORMATTED.replace("println 1;", "println 1") }
    }

    fun testUntrustedProjectRunsNothingFromTheProject() {
        val project = fixture.project
        val base = File(project.basePath!!)
        val markers = Files.createTempDirectory("groovyfmt-markers").toFile()
        val (platform, arch) = currentPlatform()
        fun plant(path: String, name: String) {
            val f = File(base, path)
            f.parentFile.mkdirs()
            f.writeText("#!/bin/sh\necho ran > '${File(markers, name)}'\necho groovyfmt 0.12.0\n")
            f.setExecutable(true)
        }
        plant("tools/groovyfmt", "setting")
        plant("node_modules/@groovyfmt/$platform-$arch/bin/groovyfmt", "native")
        plant("node_modules/.bin/groovyfmt", "launcher")
        GroovyfmtProjectSettings.get(project).path = "tools/groovyfmt"
        // nothing outside the project either: no IDE setting, and the test's PATH has no groovyfmt
        assertNull(GroovyfmtSettings.get().path)
        assertNull(findBinary(Where(null, null, false, null, System.getenv("PATH").orEmpty(), platform, arch)))

        project.setTrusted(false)
        assertFalse("the project is trusted: the test can't show anything", project.isTrusted())
        open()
        restart()
        // time for a restart that a settings read or trust check might trigger
        repeat(30) {
            PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
            Thread.sleep(100)
        }
        assertEquals(listOf<String>(), markers.list()!!.toList())
        val service = project.service<GroovyfmtService>()
        assertNull(service.binary)
        assertTrue(service.lastProblem.orEmpty(), service.lastProblem.orEmpty().contains("was not found in the path settings, node_modules or PATH"))
        assertTrue(LanguageServerManager.getInstance(project).getServerStatus(SERVER_ID) != ServerStatus.started)
        println("integration: untrusted project ran nothing from the project (${service.lastProblem})")
    }

    /** A project finds no binary and reports nothing until a groovyfmt file opens. */
    fun testNothingRunsUntilAGroovyfmtFileOpens() {
        val service = fixture.project.service<GroovyfmtService>()
        fixture.configureByText("notes.txt", "notes\n")
        repeat(20) {
            PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
            Thread.sleep(100)
        }
        assertFalse("activated by a file groovyfmt doesn't handle", service.active)
        assertNull(service.lastProblem)
        fixture.configureByText("build.gradle", "apply plugin: 'java'\n")
        waitFor("the first groovyfmt file to activate the plugin", 30_000) { service.active && service.lastProblem != null }
    }

    /**
     * A restart superseded by a later one before its UI step runs must not start the server:
     * here the second turns the plugin off while the server runs. Both resolve before the UI
     * step of either runs, since this test holds the UI thread. (With LSP4IJ 0.21.0 this also
     * passes without the plugin's staleness check: its default start options re-enable the
     * server instead of throwing. It guards the outcome, not that mechanism.)
     */
    fun testASupersededRestartDoesNotStartTheServer() {
        GroovyfmtSettings.get().path = bin()
        open()
        val service = fixture.project.service<GroovyfmtService>()
        service.restart().get(30, TimeUnit.SECONDS)
        waitFor("the server to start", 30_000) {
            LanguageServerManager.getInstance(fixture.project).getServerStatus(SERVER_ID) == ServerStatus.started
        }
        val first = service.restart()
        GroovyfmtSettings.get().enabled = false
        val second = service.restart()
        first.get(30, TimeUnit.SECONDS)
        second.get(30, TimeUnit.SECONDS)
        repeat(20) {
            PlatformTestUtil.dispatchAllEventsInIdeEventQueue()
            Thread.sleep(100)
        }
        assertNull(service.binary)
        assertTrue(LanguageServerManager.getInstance(fixture.project).getServerStatus(SERVER_ID) != ServerStatus.started)
    }

    /**
     * The server runs binary A; the path changes to B and a second restart follows before
     * either UI step runs (this test holds the UI thread, as indexing would). The first step is
     * stale and skipped; the second must still replace A with B, though its binary matches the
     * one the first restart found.
     */
    fun testAPathChangeReachesTheRunningServerThroughTwoRestarts() {
        val a = bin()
        val b = File(Files.createTempDirectory("groovyfmt-b").toFile(), "groovyfmt")
        Files.copy(File(a).toPath(), b.toPath())
        b.setExecutable(true)
        expectServerThreads(b.path)
        // the OS reports the resolved executable, so compare canonical paths
        fun serving(path: String) = ProcessHandle.allProcesses().anyMatch { p ->
            p.isAlive && p.info().command().map { File(it).canonicalPath }.orElse("") == File(path).canonicalPath &&
                p.info().arguments().orElse(arrayOf()).contentEquals(arrayOf("--lsp"))
        }
        GroovyfmtSettings.get().path = a
        open()
        val service = fixture.project.service<GroovyfmtService>()
        service.restart().get(30, TimeUnit.SECONDS)
        waitFor("the server to run A", 30_000) {
            LanguageServerManager.getInstance(fixture.project).getServerStatus(SERVER_ID) == ServerStatus.started && serving(a)
        }
        GroovyfmtSettings.get().path = b.path
        val first = service.restart()
        val second = service.restart()
        first.get(30, TimeUnit.SECONDS)
        second.get(30, TimeUnit.SECONDS)
        waitFor("the server to run B in place of A", 30_000) {
            LanguageServerManager.getInstance(fixture.project).getServerStatus(SERVER_ID) == ServerStatus.started && serving(b.path) && !serving(a)
        }
    }

    /** A binary that never reads its input can't hang Reformat with groovyfmt: the timeout kills it. */
    fun testReformatTimesOutOnABinaryThatNeverReadsItsInput() {
        val stuck = File(Files.createTempDirectory("groovyfmt-stuck").toFile(), "groovyfmt")
        stuck.writeText("#!/bin/sh\nexec sleep 60\n")
        stuck.setExecutable(true)
        // far more than a pipe holds
        val big = "println 1\n".repeat(200_000)
        val result = com.intellij.openapi.application.ApplicationManager.getApplication().executeOnPooledThread<Throwable?> {
            try {
                GroovyfmtReformatAction.format(stuck.path, big, "/tmp/A.groovy", null, timeoutMs = 2_000)
                null
            } catch (e: Throwable) {
                e
            }
        }
        val error = try {
            result.get(20, TimeUnit.SECONDS)
        } catch (e: java.util.concurrent.TimeoutException) {
            fail("format still running after 20 s against a 2 s timeout")
            null
        }
        assertTrue("$error", error is java.io.IOException && error.message!!.contains("did not finish"))
    }

    /**
     * The setup docs/editors.md describes for LSP4IJ without this plugin: a user-defined language
     * server, as LSP4IJ's New Language Server dialog creates it, running `groovyfmt --lsp` for the
     * groovyfmt file name patterns, with the client configuration that lets Reformat Code use it.
     */
    fun testUserDefinedServerWithoutThePlugin() {
        GroovyfmtSettings.get().enabled = false
        val definition = UserDefinedLanguageServerDefinition(
            "groovyfmt-user-defined", null, "groovyfmt", null, null, "${bin()} --lsp", null, emptyMap(), true,
            null, false, null, null, null, """{"format": {"existingFormatterOverrideable": true}}""", null, null,
        )
        val patterns = listOf("*.groovy", "*.gvy", "*.gy", "*.gsh", "*.gradle", "*.jenkinsfile", "Jenkinsfile", "Jenkinsfile.*")
        LanguageServersRegistry.getInstance().addServerDefinition(
            fixture.project, definition, listOf(ServerMappingSettings.createFileNamePatternsMappingSettings(patterns, "groovy")),
        )
        try {
            open()
            restart()
            assertNull(fixture.project.service<GroovyfmtService>().binary)
            waitForSemicolonDiagnostic()
            fixture.performEditorAction(IdeActions.ACTION_EDITOR_REFORMAT)
            waitFor("Reformat Code to use the user-defined server", 30_000) { fixture.editor.document.text == FORMATTED }
        } finally {
            LanguageServersRegistry.getInstance().removeServerDefinition(fixture.project, definition)
        }
    }
}
