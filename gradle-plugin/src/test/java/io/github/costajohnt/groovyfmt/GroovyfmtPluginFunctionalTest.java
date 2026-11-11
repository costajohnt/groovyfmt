package io.github.costajohnt.groovyfmt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.UUID;
import java.util.stream.Stream;
import org.gradle.testkit.runner.BuildResult;
import org.gradle.testkit.runner.GradleRunner;
import org.gradle.testkit.runner.TaskOutcome;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/**
 * Builds that apply the plugin, run with a fake groovyfmt (a shell script) so no network is needed. The fake reports
 * *.groovy files holding BAD as unformatted (format turns BAD into GOOD), LINT as a lint violation, and ERROR as an
 * error with exit code 2. Runs against the Gradle in groovyfmt.testGradleVersion, or the one building the plugin.
 */
class GroovyfmtPluginFunctionalTest {
    static final String FAKE = String.join("\n",
            "#!/bin/sh",
            "mode=format",
            "for a in \"$@\"; do case \"$a\" in --check) mode=check ;; --lint) mode=lint ;; esac; done",
            "echo \"fake groovyfmt $mode: $*\"",
            "status=0",
            "for a in \"$@\"; do",
            "  case \"$a\" in -*) continue ;; esac",
            "  for f in $(find \"$a\" -name '*.groovy' | sort); do",
            "    if grep -q ERROR \"$f\"; then echo \"error: cannot read $f\" >&2; exit 2; fi",
            "    case $mode in",
            "      check) if grep -q BAD \"$f\"; then echo \"would reformat $f\"; status=1; fi ;;",
            "      lint) if grep -q LINT \"$f\"; then echo \"$f:1:1: UnusedImport: fake violation\"; status=1; fi ;;",
            "      format) sed 's/BAD/GOOD/' \"$f\" > \"$f.tmp\" && mv \"$f.tmp\" \"$f\" ;;",
            "    esac",
            "  done",
            "done",
            "exit $status",
            "");

    @TempDir
    Path dir;

    @BeforeEach
    void setUp() throws IOException {
        write("settings.gradle", "rootProject.name = 'sample'\n");
        write("fake/groovyfmt", FAKE);
        dir.resolve("fake/groovyfmt").toFile().setExecutable(true);
        write("src/A.groovy", "GOOD\n");
    }

    void buildFile(String groovyfmtBlock) throws IOException {
        write("build.gradle", "plugins {\n    id 'base'\n    id 'io.github.costajohnt.groovyfmt'\n}\ngroovyfmt {\n" + groovyfmtBlock + "\n}\n");
    }

    void useFake() throws IOException {
        buildFile("    binary = file('fake/groovyfmt')");
    }

    @Test
    void checkRunsCheckAndLintAndPassesWhenClean() throws IOException {
        useFake();
        BuildResult r = runner("check").build();
        assertEquals(TaskOutcome.SUCCESS, r.task(":groovyfmtCheck").getOutcome());
        assertEquals(TaskOutcome.SUCCESS, r.task(":groovyfmtLint").getOutcome());
        assertNull(r.task(":groovyfmtFormat"));
        assertTrue(r.getOutput().contains("fake groovyfmt check: --check " + dir.toRealPath()) || r.getOutput().contains("fake groovyfmt check: --check " + dir), r.getOutput());
    }

    @Test
    void checkFailsWithGroovyfmtOutput() throws IOException {
        useFake();
        write("src/A.groovy", "BAD\n");
        BuildResult r = runner("check").buildAndFail();
        assertEquals(TaskOutcome.FAILED, r.task(":groovyfmtCheck").getOutcome());
        assertTrue(r.getOutput().contains("would reformat "), r.getOutput());
        assertTrue(r.getOutput().contains("need formatting. Run the groovyfmtFormat task"), r.getOutput());
    }

    @Test
    void formatRewritesFiles() throws IOException {
        useFake();
        write("src/A.groovy", "BAD\n");
        runner("groovyfmtFormat").build();
        assertEquals("GOOD\n", read("src/A.groovy"));
        runner("groovyfmtCheck").build();
    }

    @Test
    void formatThenCheckInOneBuildRunsFormatFirst() throws IOException {
        useFake();
        write("src/A.groovy", "BAD\n");
        BuildResult r = runner("groovyfmtCheck", "groovyfmtFormat").build();
        assertTrue(r.getOutput().indexOf("fake groovyfmt format") < r.getOutput().indexOf("fake groovyfmt check"), r.getOutput());
    }

    @Test
    void lintReportsViolations() throws IOException {
        useFake();
        write("src/A.groovy", "LINT\n");
        BuildResult r = runner("groovyfmtLint").buildAndFail();
        assertTrue(r.getOutput().contains("A.groovy:1:1: UnusedImport: fake violation"), r.getOutput());
        assertTrue(r.getOutput().contains("lint found the violations listed above"), r.getOutput());
    }

    @Test
    void exitCode2HasItsOwnMessage() throws IOException {
        useFake();
        write("src/A.groovy", "ERROR\n");
        BuildResult r = runner("groovyfmtCheck").buildAndFail();
        assertTrue(r.getOutput().contains("error: cannot read"), r.getOutput());
        assertTrue(r.getOutput().contains("groovyfmt could not run (exit code 2)"), r.getOutput());
        assertFalse(r.getOutput().contains("need formatting"), r.getOutput());
    }

    @Test
    void passesArgsAndPaths() throws IOException {
        buildFile("    binary = file('fake/groovyfmt')\n    args = ['-v']\n    paths = ['src']");
        write("other/B.groovy", "BAD\n");
        BuildResult r = runner("groovyfmtCheck").build();
        assertTrue(r.getOutput().contains("fake groovyfmt check: --check -v " + dir.resolve("src")), r.getOutput());
    }

    @Test
    void reusesTheConfigurationCacheAndRerunsAfterAnEdit() throws IOException {
        useFake();
        // Not -P: Gradle 7 keeps every command-line -P value in the cache entry, whoever reads it.
        write("gradle.properties", "groovyfmt.githubToken=SECRET-TOKEN-1234\n");
        BuildResult first = runner("groovyfmtCheck", "groovyfmtLint", "--configuration-cache").build();
        assertFalse(first.getOutput().contains("Reusing configuration cache"), first.getOutput());
        BuildResult second = runner("groovyfmtCheck", "groovyfmtLint", "--configuration-cache").build();
        assertTrue(second.getOutput().contains("Reusing configuration cache"), second.getOutput());
        assertEquals(TaskOutcome.SUCCESS, second.task(":groovyfmtCheck").getOutcome());

        write("src/A.groovy", "BAD\n");
        BuildResult third = runner("groovyfmtCheck", "groovyfmtLint", "--configuration-cache").buildAndFail();
        assertTrue(third.getOutput().contains("Reusing configuration cache"), third.getOutput());
        assertTrue(third.getOutput().contains("would reformat"), third.getOutput());

        assertFalse(containsBytes(dir.resolve(".gradle"), "SECRET-TOKEN-1234"), "the token is stored in the configuration cache");
    }

    @Test
    void rootProjectCoversSubprojects() throws IOException {
        write("settings.gradle", "rootProject.name = 'sample'\ninclude 'sub'\n");
        write("sub/build.gradle", "");
        write("sub/src/B.groovy", "BAD\n");
        useFake();
        BuildResult r = runner("groovyfmtCheck").buildAndFail();
        assertTrue(r.getOutput().contains("would reformat " + dir.resolve("sub/src/B.groovy")), r.getOutput());
        assertNull(r.task(":sub:groovyfmtCheck"));
    }

    @Test
    void downloadsVerifiesCachesAndRunsOffline() throws Exception {
        String version = uniqueVersion();
        Path release = release(version, false);
        buildFile("    version = '" + version + "'");
        BuildResult r = runner("groovyfmtCheck", "-Pgroovyfmt.internal.downloadBaseUrl=" + release.toUri()).build();
        assertTrue(r.getOutput().contains("Downloading groovyfmt v" + version), r.getOutput());
        assertTrue(r.getOutput().contains("fake groovyfmt check"), r.getOutput());

        deleteTree(release);
        BuildResult offline = runner("groovyfmtCheck", "--offline").build();
        assertFalse(offline.getOutput().contains("Downloading"), offline.getOutput());
        assertTrue(offline.getOutput().contains("fake groovyfmt check"), offline.getOutput());
    }

    @Test
    void offlineWithoutACachedBinaryFails() throws IOException {
        buildFile("    version = '" + uniqueVersion() + "'");
        BuildResult r = runner("groovyfmtCheck", "--offline").buildAndFail();
        assertTrue(r.getOutput().contains("is not in the cache"), r.getOutput());
        assertTrue(r.getOutput().contains("Gradle is offline"), r.getOutput());
    }

    @Test
    void rejectsATamperedArchive() throws Exception {
        String version = uniqueVersion();
        Path release = release(version, true);
        buildFile("    version = '" + version + "'");
        BuildResult r = runner("groovyfmtCheck", "-Pgroovyfmt.internal.downloadBaseUrl=" + release.toUri()).buildAndFail();
        assertTrue(r.getOutput().contains("checksum mismatch"), r.getOutput());
        assertFalse(r.getOutput().contains("fake groovyfmt"), r.getOutput());
        File cached = new File(testKitDir(), "groovyfmt/" + version + "/" + target() + "/groovyfmt");
        assertFalse(cached.exists());
    }

    // --- helpers

    GradleRunner runner(String... args) {
        GradleRunner r = GradleRunner.create()
                .withProjectDir(dir.toFile())
                .withPluginClasspath()
                .withTestKitDir(testKitDir())
                .withArguments(Stream.concat(Arrays.stream(args), Stream.of("--stacktrace")).toArray(String[]::new));
        String version = System.getProperty("groovyfmt.testGradleVersion");
        return version == null ? r : r.withGradleVersion(version);
    }

    static File testKitDir() {
        return new File(System.getProperty("groovyfmt.testKitDir"));
    }

    static String target() {
        return Provisioner.target(System.getProperty("os.name"), System.getProperty("os.arch"));
    }

    // Each test downloads its own version, so the shared TestKit cache never answers for it.
    static String uniqueVersion() {
        return "0.0.1-t" + UUID.randomUUID().toString().replace("-", "").substring(0, 12);
    }

    /** A local release directory, <dir>/v<version>/<archive> and its .sha256, holding the fake binary. */
    Path release(String version, boolean tamper) throws Exception {
        String name = "groovyfmt-v" + version + "-" + target();
        Path tagDir = Files.createDirectories(dir.resolve("release/v" + version));
        byte[] archive = ProvisionerTest.tarGz(name + "/README.md", '0', "readme", name + "/groovyfmt", '0', FAKE);
        Files.write(tagDir.resolve(name + ".tar.gz"), archive);
        String sum = ProvisionerTest.sha256(tamper ? ProvisionerTest.tarGz(name + "/groovyfmt", '0', "something else") : archive);
        write("release/v" + version + "/" + name + ".tar.gz.sha256", sum + "  " + name + ".tar.gz\n");
        return dir.resolve("release");
    }

    void write(String path, String content) throws IOException {
        Path p = dir.resolve(path);
        Files.createDirectories(p.getParent());
        Files.writeString(p, content);
    }

    String read(String path) throws IOException {
        return Files.readString(dir.resolve(path));
    }

    static void deleteTree(Path root) throws IOException {
        try (Stream<Path> files = Files.walk(root)) {
            files.sorted((a, b) -> b.compareTo(a)).forEach(p -> p.toFile().delete());
        }
    }

    static boolean containsBytes(Path root, String needle) throws IOException {
        try (Stream<Path> files = Files.walk(root)) {
            return files.filter(Files::isRegularFile).anyMatch(p -> {
                try {
                    return new String(Files.readAllBytes(p), StandardCharsets.ISO_8859_1).contains(needle);
                } catch (IOException e) {
                    throw new java.io.UncheckedIOException(e);
                }
            });
        }
    }
}
