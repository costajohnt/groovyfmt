package io.github.costajohnt.groovyfmt.intellij

import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import org.junit.AfterClass
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeFalse
import org.junit.BeforeClass
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.net.InetSocketAddress
import java.nio.file.Files
import java.nio.file.Path
import java.time.Duration
import java.util.concurrent.CopyOnWriteArrayList
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

private val windows = File.separatorChar == '\\'
private val root = System.getProperty("user.name") == "root"
private val tmp: Path = Files.createTempDirectory("groovyfmt-jetbrains-")

private fun fakeBin(p: File): String {
    p.parentFile.mkdirs()
    p.writeText("#!/bin/sh\necho groovyfmt 0.12.0\n")
    p.setExecutable(true)
    return p.path
}

private fun assertThrows(pattern: String, f: () -> Unit) {
    try {
        f()
    } catch (e: Exception) {
        assertTrue("${e.message} does not match $pattern", Regex(pattern).containsMatchIn(e.toString()))
        return
    }
    fail("no exception matching $pattern")
}

/** A .tar.gz made by tar(1), laid out as the release archives are: <stem>/groovyfmt and friends. */
private fun makeTarGz(stem: String, bin: ByteArray): ByteArray {
    val dir = Files.createTempDirectory(tmp, "tar-").toFile()
    File(dir, stem).mkdirs()
    File(dir, "$stem/README.md").writeText("readme")
    File(dir, "$stem/groovyfmt").writeBytes(bin)
    val p = ProcessBuilder("tar", "-czf", "a.tar.gz", stem).directory(dir).inheritIO().start()
    assertEquals(0, p.waitFor())
    return File(dir, "a.tar.gz").readBytes()
}

private fun makeZip(vararg entries: Pair<String, ByteArray>): ByteArray {
    val out = ByteArrayOutputStream()
    ZipOutputStream(out).use { z ->
        for ((name, data) in entries) {
            z.putNextEntry(ZipEntry(name))
            z.write(data)
            z.closeEntry()
        }
    }
    return out.toByteArray()
}

class FindBinaryTest {
    private val ws = File(tmp.toFile(), "ws")
    private val pathDir = File(tmp.toFile(), "pathbin")
    private val onPath = File(pathDir, "groovyfmt").path
    private val platform = currentPlatform().first
    private val arch = currentPlatform().second
    private val native = File(ws, "node_modules/@groovyfmt/$platform-$arch/bin/groovyfmt")
    private val launcher = File(ws, "node_modules/.bin/groovyfmt")
    // what a hostile repository would ship for a relative path setting to land on
    private val planted = File(ws, "bin/groovyfmt")

    init {
        assumeFalse("uses Unix executables", windows)
        fakeBin(native)
        fakeBin(launcher)
        fakeBin(File(onPath))
        fakeBin(planted)
        // where the relative PATH entry `rel` would resolve from the project directory
        fakeBin(File(ws, "rel/groovyfmt"))
    }

    private fun where(
        appPath: String? = null,
        projectPath: String? = null,
        trusted: Boolean = true,
        projectDir: String? = ws.path,
        envPath: String = listOf("rel", pathDir.path).joinToString(File.pathSeparator),
    ) = Where(appPath, projectPath, trusted, projectDir, envPath, platform, arch)

    @Test
    fun `the path setting comes first, the project's over the IDE's`() {
        assertEquals(Found("/w/groovyfmt", "setting"), findBinary(where(appPath = "/u/groovyfmt", projectPath = "/w/groovyfmt")))
        assertEquals(Found("/u/groovyfmt", "setting"), findBinary(where(appPath = "/u/groovyfmt")))
        assertEquals(Found("/u/groovyfmt", "setting"), findBinary(where(appPath = " /u/groovyfmt ", projectPath = "  ")))
    }

    @Test
    fun `trusted, a relative project path is resolved against the project directory`() {
        assertEquals(Found(planted.path, "setting"), findBinary(where(projectPath = "bin/groovyfmt")))
        assertEquals(Found(planted.path, "setting"), findBinary(where(projectPath = "./x/../bin/groovyfmt")))
        val r = findBinary(where(projectPath = "bin/groovyfmt", projectDir = null))
        assertTrue(r is Problem && r.message.contains("no directory"))
    }

    @Test
    fun `untrusted, a relative IDE path never resolves into the project`() {
        for (app in listOf("bin/groovyfmt", "./bin/groovyfmt", "groovyfmt")) {
            val r = findBinary(where(trusted = false, appPath = app))
            assertTrue("$app: $r", r is Problem && r.message.contains("must be an absolute path"))
        }
    }

    @Test
    fun `trusted, a relative IDE path is refused too`() {
        assertTrue(findBinary(where(appPath = "bin/groovyfmt")) is Problem)
    }

    @Test
    fun `untrusted, a project path of any kind is ignored`() {
        for (project in listOf("/w/groovyfmt", "bin/groovyfmt", planted.path)) {
            assertEquals(Found(onPath, "PATH"), findBinary(where(trusted = false, projectPath = project)))
            assertEquals(Found("/u/groovyfmt", "setting"), findBinary(where(trusted = false, projectPath = project, appPath = "/u/groovyfmt")))
        }
    }

    @Test
    fun `then the npm package native binary, then its launcher`() {
        assertEquals(Found(native.path, "node_modules"), findBinary(where()))
        native.delete()
        try {
            assertEquals(Found(launcher.path, "node_modules"), findBinary(where()))
        } finally {
            fakeBin(native)
        }
    }

    @Test
    fun `untrusted, node_modules is ignored`() {
        assertEquals(Found(onPath, "PATH"), findBinary(where(trusted = false)))
    }

    @Test
    fun `then PATH, skipping relative entries`() {
        assertEquals(Found(onPath, "PATH"), findBinary(where(projectDir = null)))
        assertNull(findBinary(where(projectDir = null, envPath = "rel")))
    }

    @Test
    fun `a file without the execute bit does not count`() {
        val plain = File(tmp.toFile(), "plain")
        plain.mkdirs()
        File(plain, "groovyfmt").writeText("x")
        assertNull(findBinary(where(projectDir = null, envPath = plain.path)))
    }

    @Test
    fun `nothing found`() {
        assertNull(findBinary(where(projectDir = null, envPath = "")))
    }
}

class VersionAndChecksumTest {
    @Test
    fun `parseVersion and isOlder`() {
        assertEquals(listOf(0, 12, 0), parseVersion("groovyfmt 0.12.0\n"))
        assertEquals(listOf(1, 2, 3), parseVersion("groovyfmt 1.2.3-rc.1"))
        assertNull(parseVersion("ruff 0.1.0"))
        assertNull(parseVersion(""))
        assertTrue(isOlder(listOf(0, 8, 0), listOf(0, 9, 0)))
        assertFalse(isOlder(listOf(0, 9, 0), listOf(0, 9, 0)))
        assertFalse(isOlder(listOf(0, 10, 0), listOf(0, 9, 0)))
        assertFalse(isOlder(listOf(1, 0, 0), listOf(0, 9, 9)))
    }

    @Test
    fun `runVersion runs the binary`() {
        assumeFalse(windows)
        assertEquals("groovyfmt 0.12.0", runVersion(fakeBin(File(tmp.toFile(), "v/groovyfmt"))).trim())
        assertThrows("IOException") { runVersion(File(tmp.toFile(), "missing").path) }
    }

    @Test
    fun verifySha256() {
        val b = "binary".toByteArray()
        verifySha256(b, sha256(b), "a")
        assertThrows("checksum mismatch for a") { verifySha256(b, "0".repeat(64), "a") }
        assertThrows("no SHA-256 digest") { verifySha256(b, "", "a") }
        assertThrows("no SHA-256 digest") { verifySha256(b, "abc", "a") }
    }

    @Test
    fun `checksumsOf hashes each archive and checks its sha256 file`() {
        val dir = Files.createTempDirectory(tmp, "sums-")
        val expected = mutableMapOf<String, String>()
        for (key in TARGETS.keys) {
            val (platform, arch) = key.split('-')
            val name = archiveName("1.2.3", platform, arch)!!
            val bytes = "archive for $key".toByteArray()
            Files.write(dir.resolve(name), bytes)
            Files.writeString(dir.resolve("$name.sha256"), "${sha256(bytes)}  $name\n")
            expected[key] = sha256(bytes)
        }
        val sums = checksumsOf("1.2.3", dir)
        assertEquals("1.2.3", sums.getProperty("version"))
        for ((k, v) in expected) assertEquals(v, sums.getProperty(k))
        assertEquals(6, sums.size)
        assertEquals(expected["linux-x64"], embeddedSha256(sums, "1.2.3", "linux", "x64"))
        assertNull(embeddedSha256(sums, "1.2.4", "linux", "x64"))
        assertNull(embeddedSha256(null, "1.2.3", "linux", "x64"))
        assertEquals("groovyfmt-v1.2.3-x86_64-pc-windows-gnu.zip", archiveName("1.2.3", "win32", "x64"))

        // the file written by the release build reads back the same
        val out = dir.resolve("out.properties")
        main(arrayOf("v1.2.3", dir.toString(), out.toString()))
        val read = java.util.Properties().apply { Files.newInputStream(out).use { load(it) } }
        assertEquals(sums, read)

        Files.writeString(dir.resolve("${archiveName("1.2.3", "darwin", "x64")}.sha256"), "${"0".repeat(64)}  x\n")
        assertThrows("""\.sha256 says 0+, the archive hashes to""") { checksumsOf("1.2.3", dir) }
        assertThrows("NoSuchFile") { checksumsOf("9.9.9", dir) }
    }
}

class ArchiveTest {
    @Test
    fun `untarOne reads one entry of a tar archive`() {
        assumeFalse("needs tar", windows)
        val bin = ByteArray(3000) { 7 }
        val tgz = makeTarGz("groovyfmt-v1.0.0-x", bin)
        assertArrayEquals(bin, untarOne(tgz, "groovyfmt-v1.0.0-x/groovyfmt"))
        assertThrows("no groovyfmt-v1.0.0-x/other") { untarOne(tgz, "groovyfmt-v1.0.0-x/other") }
        // a directory entry is not a file
        assertThrows("no groovyfmt-v1.0.0-x in the archive") { untarOne(tgz, "groovyfmt-v1.0.0-x") }
        assertThrows("GZIP") { untarOne("not gzip".toByteArray(), "x") }
    }

    @Test
    fun `unzipOne reads stored and deflated entries`() {
        val a = "stored contents".toByteArray()
        val b = ByteArray(5000) { 'z'.code.toByte() }
        val zip = makeZip("d/a" to a, "d/groovyfmt.exe" to b, "d/" to ByteArray(0))
        assertArrayEquals(a, unzipOne(zip, "d/a"))
        assertArrayEquals(b, unzipOne(zip, "d/groovyfmt.exe"))
        assertThrows("no d/missing") { unzipOne(zip, "d/missing") }
        assertThrows("not a zip") { unzipOne("nope".toByteArray(), "x") }
    }
}

class DownloadTest {
    companion object {
        private val files = mutableMapOf<String, (HttpExchange) -> Unit>()
        private val fetched = CopyOnWriteArrayList<String>()
        private lateinit var server: HttpServer
        private lateinit var base: String
        private val bin = "#!/bin/sh\necho groovyfmt 1.0.0\n".toByteArray()
        private lateinit var real: Map<String, String>
        private val zipBin = ByteArray(4000) { 'w'.code.toByte() }
        private const val LINUX = "groovyfmt-v1.0.0-x86_64-unknown-linux-musl.tar.gz"
        private const val ARM = "groovyfmt-v1.0.0-aarch64-unknown-linux-musl.tar.gz"
        private const val WIN = "groovyfmt-v1.0.0-x86_64-pc-windows-gnu.zip"

        private fun bytes(b: ByteArray): (HttpExchange) -> Unit = { ex ->
            ex.sendResponseHeaders(200, b.size.toLong())
            ex.responseBody.use { it.write(b) }
        }

        private fun serve(name: String, body: (HttpExchange) -> Unit, sum: ByteArray? = null) {
            files["/v1.0.0/$name"] = body
            // the release's own checksum file, which download must not rely on
            if (sum != null) files["/v1.0.0/$name.sha256"] = bytes("${sha256(sum)}  $name\n".toByteArray())
        }

        @BeforeClass
        @JvmStatic
        fun start() {
            assumeFalse("needs tar", windows)
            val tgz = makeTarGz("groovyfmt-v1.0.0-x86_64-unknown-linux-musl", bin)
            val evil = makeTarGz("groovyfmt-v1.0.0-aarch64-unknown-linux-musl", "#!/bin/sh\necho evil\n".toByteArray())
            val zip = makeZip("groovyfmt-v1.0.0-x86_64-pc-windows-gnu/groovyfmt.exe" to zipBin)
            // what the plugin carries for each archive
            real = mapOf(
                "linux" to sha256(tgz),
                "arm" to sha256(makeTarGz("groovyfmt-v1.0.0-aarch64-unknown-linux-musl", bin)),
                "win" to sha256(zip),
            )
            serve(LINUX, bytes(tgz), tgz)
            serve(WIN, bytes(zip), zip)
            // the release was rewritten: another archive, with a .sha256 that matches it
            serve(ARM, bytes(evil), evil)
            serve("groovyfmt-v1.0.0-x86_64-apple-darwin.tar.gz", { ex ->
                ex.sendResponseHeaders(200, 100L * 1024 * 1024)
                ex.responseBody.write(ByteArray(1024))
                ex.responseBody.flush()
            })
            serve("groovyfmt-v1.0.0-aarch64-apple-darwin.tar.gz", { ex ->
                // no length, and it never ends
                ex.sendResponseHeaders(200, 0)
                val chunk = ByteArray(64 * 1024)
                try {
                    while (true) ex.responseBody.write(chunk)
                } catch (e: IOException) {
                    // the client gave up
                }
            })
            files["/v2.0.0/stall"] = { ex ->
                ex.sendResponseHeaders(200, 1000)
                ex.responseBody.write(ByteArray(10))
                ex.responseBody.flush()
                Thread.sleep(30_000)
            }
            server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
            server.executor = java.util.concurrent.Executors.newCachedThreadPool { r -> Thread(r).apply { isDaemon = true } }
            server.createContext("/") { ex ->
                fetched.add(ex.requestURI.path)
                val h = files[ex.requestURI.path]
                if (h == null) {
                    ex.sendResponseHeaders(404, -1)
                    ex.close()
                } else {
                    h(ex)
                }
            }
            server.start()
            base = "http://127.0.0.1:${server.address.port}"
        }

        @AfterClass
        @JvmStatic
        fun stop() {
            if (::server.isInitialized) server.stop(0)
        }
    }

    private fun store(platform: String, arch: String) = downloadPath(Files.createTempDirectory(tmp, "store-"), "1.0.0", platform, arch)

    private fun listing(dest: Path) = dest.parent.toFile().list()!!.toList()

    @Test
    fun `writes the binary that matches the embedded checksum, executable`() {
        fetched.clear()
        val dest = store("linux", "x64")
        download("1.0.0", "linux", "x64", dest, real["linux"], base)
        assertArrayEquals(bin, Files.readAllBytes(dest))
        assertTrue(dest.toFile().canExecute())
        assertEquals(listOf("groovyfmt"), listing(dest))
        // the release's .sha256 is never fetched
        assertEquals(listOf("/v1.0.0/$LINUX"), fetched.toList())
    }

    @Test
    fun `the Windows zip`() {
        val dest = store("win32", "x64")
        download("1.0.0", "win32", "x64", dest, real["win"], base)
        assertArrayEquals(zipBin, Files.readAllBytes(dest))
        assertTrue(dest.toString().endsWith("groovyfmt.exe"))
    }

    @Test
    fun `a swapped archive with a matching swapped sha256 file is rejected`() {
        val dest = store("linux", "arm64")
        assertThrows("checksum mismatch") { download("1.0.0", "linux", "arm64", dest, real["arm"], base) }
        assertFalse(Files.exists(dest.parent))
    }

    @Test
    fun `no embedded checksum, nothing is fetched`() {
        fetched.clear()
        assertThrows("no checksum") { download("1.0.0", "linux", "x64", store("linux", "x64"), null, base) }
        assertEquals(listOf<String>(), fetched.toList())
    }

    @Test
    fun `a missing release`() {
        assertThrows("HTTP 404") { download("2.0.0", "linux", "x64", store("linux", "x64"), real["linux"], base) }
    }

    @Test
    fun `a platform without a prebuilt binary`() {
        assertThrows("no prebuilt binary for freebsd-x64") { download("1.0.0", "freebsd", "x64", store("freebsd", "x64"), real["linux"], base) }
    }

    @Test(timeout = 10_000)
    fun `a content-length over the cap`() {
        assertThrows("larger than") { download("1.0.0", "darwin", "x64", store("darwin", "x64"), real["linux"], base, maxBytes = 1024 * 1024) }
    }

    @Test(timeout = 10_000)
    fun `a body that keeps coming past the cap`() {
        assertThrows("larger than") { download("1.0.0", "darwin", "arm64", store("darwin", "arm64"), real["linux"], base, maxBytes = 1024 * 1024) }
    }

    @Test(timeout = 10_000)
    fun `a stalled response times out`() {
        assertThrows("timed out") {
            download("1.0.0", "linux", "x64", store("linux", "x64"), real["linux"], "$base/v2.0.0/stall#", timeout = Duration.ofMillis(500))
        }
    }

    @Test
    fun `a failed rename leaves no temp file`() {
        val dest = store("linux", "x64")
        Files.createDirectories(dest.resolve("occupied"))
        assertThrows("Is a directory|not empty") { download("1.0.0", "linux", "x64", dest, real["linux"], base) }
        assertEquals(listOf("groovyfmt"), listing(dest))
    }

    @Test
    fun `the same binary already in place is used when the write fails`() {
        assumeFalse("root ignores permissions", root)
        val dest = store("linux", "x64")
        Files.createDirectories(dest.parent)
        Files.write(dest, bin)
        dest.parent.toFile().setWritable(false)
        try {
            download("1.0.0", "linux", "x64", dest, real["linux"], base)
            // a different file there is not taken for it
            dest.parent.toFile().setWritable(true)
            Files.writeString(dest, "other")
            dest.parent.toFile().setWritable(false)
            assertThrows("AccessDenied") { download("1.0.0", "linux", "x64", dest, real["linux"], base) }
        } finally {
            dest.parent.toFile().setWritable(true)
        }
        assertEquals(listOf("groovyfmt"), listing(dest))
    }
}
