// Finding, checking and downloading the groovyfmt binary. Nothing here uses the IntelliJ
// Platform, so the unit tests run it on a plain JVM. The rules are the VS Code extension's
// (editors/vscode/src/binary.ts).
package io.github.costajohnt.groovyfmt.intellij

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.time.Duration
import java.util.Properties
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.zip.GZIPInputStream
import java.util.zip.ZipInputStream

/** The first release with `--lsp`. */
const val MIN_LSP_VERSION = "0.9.0"
const val RELEASES = "https://github.com/costajohnt/groovyfmt/releases/download"

/** The release asset target for each platform, keyed as Node names platforms (npm/build.mjs). */
val TARGETS = mapOf(
    "darwin-arm64" to "aarch64-apple-darwin",
    "darwin-x64" to "x86_64-apple-darwin",
    "linux-arm64" to "aarch64-unknown-linux-musl",
    "linux-x64" to "x86_64-unknown-linux-musl",
    "win32-x64" to "x86_64-pc-windows-gnu",
)

fun exeName(platform: String) = if (platform == "win32") "groovyfmt.exe" else "groovyfmt"

/** This JVM's platform and architecture, named as Node names them: ("linux", "x64"). */
fun currentPlatform(): Pair<String, String> {
    val os = System.getProperty("os.name").lowercase()
    val platform = when {
        os.startsWith("mac") || os.startsWith("darwin") -> "darwin"
        os.startsWith("windows") -> "win32"
        os.startsWith("linux") -> "linux"
        else -> os
    }
    val arch = when (val a = System.getProperty("os.arch").lowercase()) {
        "amd64", "x86_64" -> "x64"
        "aarch64", "arm64" -> "arm64"
        else -> a
    }
    return platform to arch
}

sealed interface Resolution

/** A binary to run, and where it came from: setting, node_modules, PATH or download. */
data class Found(val path: String, val source: String) : Resolution

/** No binary to run, and why; the caller reports it and offers no download. */
data class Problem(val message: String) : Resolution

data class Where(
    /** The path setting from the IDE's application settings. */
    val appPath: String?,
    /** The path setting from the project's settings (.idea/groovyfmt.xml). */
    val projectPath: String?,
    val trusted: Boolean,
    val projectDir: String?,
    val envPath: String,
    val platform: String,
    val arch: String,
)

private fun isAbsolute(p: String) = File(p).isAbsolute

private fun isExecutable(p: File, platform: String) = p.isFile && (platform == "win32" || p.canExecute())

/**
 * The binary to run, in order: the project's path setting, the application's path setting, the
 * project's node_modules, PATH. The project's setting and node_modules count only in a trusted
 * project. `null` when none applies (the caller may then offer a download). A path setting that
 * names no file is still returned, so the error names the path the user set.
 *
 * A relative path is resolved against the project directory only when it comes from the
 * project's own settings in a trusted project. A relative path in the application settings is
 * refused: it would resolve into whatever project is open, trusted or not.
 */
fun findBinary(w: Where): Resolution? {
    val project = if (w.trusted) w.projectPath?.trim()?.ifEmpty { null } else null
    if (project != null) {
        if (isAbsolute(project)) return Found(project, "setting")
        val base = w.projectDir
            ?: return Problem("The groovyfmt path \"$project\" in the project settings is relative and the project has no directory to resolve it against.")
        return Found(File(base, project).toPath().normalize().toString(), "setting")
    }
    val app = w.appPath?.trim()?.ifEmpty { null }
    if (app != null) {
        if (isAbsolute(app)) return Found(app, "setting")
        return Problem("The groovyfmt path in the IDE settings must be an absolute path, and \"$app\" is not. Set the full path to the binary.")
    }
    val exe = exeName(w.platform)
    if (w.trusted && w.projectDir != null) {
        // the npm package's native binary, run directly, then its launcher (a Node script)
        // ponytail: no .bin/groovyfmt.cmd on Windows (running it needs a shell); the native
        // binary is there whenever the package installed properly
        val nm = File(w.projectDir, "node_modules")
        val candidates = listOfNotNull(
            File(nm, "@groovyfmt/${w.platform}-${w.arch}/bin/$exe"),
            if (w.platform != "win32") File(nm, ".bin/groovyfmt") else null,
        )
        candidates.firstOrNull { isExecutable(it, w.platform) }?.let { return Found(it.path, "node_modules") }
    }
    for (dir in w.envPath.split(File.pathSeparatorChar)) {
        // a relative PATH entry would resolve inside the project
        if (dir.isEmpty() || !isAbsolute(dir)) continue
        val p = File(dir, exe)
        if (isExecutable(p, w.platform)) return Found(p.path, "PATH")
    }
    return null
}

/** [major, minor, patch] from `groovyfmt --version` output, such as "groovyfmt 0.12.0". */
fun parseVersion(out: String): List<Int>? =
    Regex("""^groovyfmt (\d+)\.(\d+)\.(\d+)""").find(out.trim())?.groupValues?.drop(1)?.map { it.toInt() }

fun isOlder(a: List<Int>, b: List<Int>): Boolean {
    for (i in 0..2) if (a[i] != b[i]) return a[i] < b[i]
    return false
}

/** The binary's `--version` output; throws if it can't be run within 10 seconds. */
fun runVersion(path: String): String {
    val p = ProcessBuilder(path, "--version").redirectErrorStream(true).start()
    val out = CompletableFuture.supplyAsync { p.inputStream.readBytes().toString(Charsets.UTF_8) }
    if (!p.waitFor(10, TimeUnit.SECONDS)) {
        p.destroyForcibly()
        throw IOException("$path --version did not finish within 10 seconds")
    }
    val text = out.get(10, TimeUnit.SECONDS)
    if (p.exitValue() != 0) throw IOException("$path --version exited with ${p.exitValue()}: ${text.trim()}")
    return text
}

fun sha256(b: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }

/** Throws unless `bytes` hash to `expected`, a SHA-256 digest in hex. */
fun verifySha256(bytes: ByteArray, expected: String, name: String) {
    if (!Regex("^[0-9a-f]{64}$").matches(expected)) throw IOException("no SHA-256 digest for $name")
    val actual = sha256(bytes)
    if (actual != expected) throw IOException("checksum mismatch for $name: expected $expected, got $actual")
}

/** The release archive's file name, such as groovyfmt-v0.12.0-x86_64-unknown-linux-musl.tar.gz. */
fun archiveName(version: String, platform: String, arch: String): String? =
    TARGETS["$platform-$arch"]?.let { "groovyfmt-v$version-$it.${if (platform == "win32") "zip" else "tar.gz"}" }

/**
 * Checksums for every platform's archive in `dir`, hashed from the archives themselves, as a
 * release build of the plugin carries them (groovyfmt-checksums.properties): `version`, and the
 * SHA-256 of each archive by `<platform>-<arch>`. Each archive's `.sha256` must agree, as a check
 * that `dir` holds what the build job produced.
 */
fun checksumsOf(version: String, dir: Path): Properties {
    val out = Properties()
    out["version"] = version
    for (key in TARGETS.keys) {
        val (platform, arch) = key.split('-')
        val name = archiveName(version, platform, arch)!!
        val digest = sha256(Files.readAllBytes(dir.resolve(name)))
        val listed = Files.readString(dir.resolve("$name.sha256")).trim().split(Regex("""\s"""))[0]
        if (listed != digest) throw IOException("$name.sha256 says $listed, the archive hashes to $digest")
        out[key] = digest
    }
    return out
}

/** The embedded checksum for this version and platform, if the plugin carries one. */
fun embeddedSha256(c: Properties?, version: String, platform: String, arch: String): String? =
    if (c?.getProperty("version") == version) c.getProperty("$platform-$arch") else null

/** The contents of the regular file `name` in a .tar.gz. Nothing in the archive becomes a path. */
fun untarOne(tgz: ByteArray, name: String): ByteArray {
    GZIPInputStream(ByteArrayInputStream(tgz)).use { input ->
        val h = ByteArray(512)
        while (input.readNBytes(h, 0, 512) == 512) {
            if (h.all { it == 0.toByte() }) break
            fun field(off: Int, len: Int): String {
                var end = off
                while (end < off + len && h[end] != 0.toByte()) end++
                return String(h, off, end - off, Charsets.UTF_8)
            }
            val prefix = field(345, 155)
            val path = (if (prefix.isEmpty()) "" else "$prefix/") + field(0, 100)
            val sizeField = field(124, 12).trim()
            if (!Regex("[0-7]*").matches(sizeField)) throw IOException("corrupt tar header")
            val size = if (sizeField.isEmpty()) 0L else sizeField.toLong(8)
            val type = h[156]
            if (path == name && (type == '0'.code.toByte() || type == 0.toByte())) {
                val data = input.readNBytes(Math.toIntExact(size))
                if (data.size.toLong() != size) throw IOException("truncated archive")
                return data
            }
            // ponytail: pax and GNU long-name headers are skipped as ordinary entries; the release
            // archives' names are short, so the plain ustar name is always the real one
            input.skipNBytes((size + 511) / 512 * 512)
        }
    }
    throw IOException("no $name in the archive")
}

/** The contents of the file `name` in a zip archive. Nothing in the archive becomes a path. */
fun unzipOne(zip: ByteArray, name: String): ByteArray {
    ZipInputStream(ByteArrayInputStream(zip)).use { input ->
        var any = false
        while (true) {
            val e = input.nextEntry ?: break
            any = true
            // a directory entry's name ends in `/`, so it never matches
            if (e.name == name) return input.readAllBytes()
        }
        if (!any) throw IOException("not a zip archive")
    }
    throw IOException("no $name in the archive")
}

/** Where a downloaded binary for this version and platform lives under `storage`. */
fun downloadPath(storage: Path, version: String, platform: String, arch: String): Path =
    storage.resolve(version).resolve("$platform-$arch").resolve(exeName(platform))

/** The release archives are a few MB; anything this large is not one. */
const val MAX_ARCHIVE_BYTES = 64L * 1024 * 1024

/** `url`'s body, failing past `timeout` (the whole transfer) or `maxBytes`. */
fun fetchCapped(url: String, timeout: Duration, maxBytes: Long): ByteArray {
    // ponytail: redirects are followed to any https host (GitHub serves release assets from
    // another one); the bytes are trusted only after they match the embedded checksum
    val client = HttpClient.newBuilder().followRedirects(HttpClient.Redirect.NORMAL).connectTimeout(timeout).build()
    // A stalled body would block a read forever: at the deadline the client is shut down,
    // which fails the read.
    val timedOut = AtomicBoolean()
    CompletableFuture.delayedExecutor(timeout.toMillis(), TimeUnit.MILLISECONDS).execute {
        timedOut.set(true)
        client.shutdownNow()
    }
    try {
        val req = HttpRequest.newBuilder(URI(url)).timeout(timeout).header("User-Agent", "groovyfmt-jetbrains").build()
        val res = client.send(req, HttpResponse.BodyHandlers.ofInputStream())
        res.body().use { body ->
            if (res.statusCode() != 200) throw IOException("$url: HTTP ${res.statusCode()}")
            if (res.headers().firstValueAsLong("content-length").orElse(-1) > maxBytes) throw IOException("$url is larger than $maxBytes bytes")
            val out = ByteArrayOutputStream()
            val buf = ByteArray(65536)
            while (true) {
                val n = body.read(buf)
                if (n < 0) break
                if (out.size() + n > maxBytes) throw IOException("$url is larger than $maxBytes bytes")
                out.write(buf, 0, n)
            }
            return out.toByteArray()
        }
    } catch (e: IOException) {
        if (timedOut.get()) throw IOException("$url: timed out after ${timeout.seconds} seconds", e)
        throw e
    } finally {
        client.shutdownNow()
    }
}

/**
 * Downloads the release archive for `version`, checks it against `sha256` (the checksum the
 * plugin carries, never one fetched with the archive), and writes the binary to `dest`. Nothing
 * is written unless the checksum matches.
 */
fun download(
    version: String,
    platform: String,
    arch: String,
    dest: Path,
    sha256: String?,
    base: String = RELEASES,
    timeout: Duration = Duration.ofMinutes(2),
    maxBytes: Long = MAX_ARCHIVE_BYTES,
) {
    val archive = archiveName(version, platform, arch) ?: throw IOException("groovyfmt has no prebuilt binary for $platform-$arch")
    if (sha256 == null) throw IOException("no checksum for $archive in this build of the plugin")
    val bytes = fetchCapped("$base/v$version/$archive", timeout, maxBytes)
    verifySha256(bytes, sha256, archive)
    val entry = "${archive.removeSuffix(".zip").removeSuffix(".tar.gz")}/${exeName(platform)}"
    val bin = if (platform == "win32") unzipOne(bytes, entry) else untarOne(bytes, entry)
    Files.createDirectories(dest.parent)
    // written whole, then renamed, so a half-written binary is never at `dest`
    val tmp = dest.resolveSibling("${dest.fileName}.${ProcessHandle.current().pid()}.${System.nanoTime()}.tmp")
    try {
        Files.write(tmp, bin)
        tmp.toFile().setExecutable(true)
        Files.move(tmp, dest, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
    } catch (e: IOException) {
        Files.deleteIfExists(tmp)
        // Another IDE window may have put the same binary there first (on Windows a running
        // .exe can't be replaced). The same bytes as the verified archive's binary are as good.
        val same = try { Files.readAllBytes(dest).contentEquals(bin) } catch (missing: IOException) { false }
        if (!same) throw e
    }
}

/**
 * Writes the checksums a release build carries, from the release archives the same workflow
 * run built: `<version> <archives-dir> <out-file>`. The Gradle task groovyfmtChecksums runs it.
 */
fun main(args: Array<String>) {
    require(args.size == 3) { "usage: <version> <archives-dir> <out-file>" }
    val sums = checksumsOf(args[0].removePrefix("v"), Path.of(args[1]))
    Files.newOutputStream(Path.of(args[2])).use { sums.store(it, null) }
    println("${args[2]}: ${sums.size - 1} archives for ${sums["version"]}")
}
