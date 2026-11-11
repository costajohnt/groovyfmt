package io.github.costajohnt.groovyfmt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.zip.GZIPOutputStream;
import org.gradle.api.GradleException;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class ProvisionerTest {
    @TempDir
    Path tmp;

    @Test
    void mapsPlatformsToReleaseTargets() {
        assertEquals("x86_64-unknown-linux-musl", Provisioner.target("Linux", "amd64"));
        assertEquals("aarch64-unknown-linux-musl", Provisioner.target("Linux", "aarch64"));
        assertEquals("x86_64-apple-darwin", Provisioner.target("Mac OS X", "x86_64"));
        assertEquals("aarch64-apple-darwin", Provisioner.target("Mac OS X", "aarch64"));
    }

    @Test
    void rejectsWindowsAndUnknownPlatforms() {
        assertTrue(assertThrows(GradleException.class, () -> Provisioner.target("Windows 11", "amd64"))
                .getMessage().contains("no Windows release"));
        assertTrue(assertThrows(GradleException.class, () -> Provisioner.target("FreeBSD", "amd64"))
                .getMessage().contains("FreeBSD/amd64"));
        assertTrue(assertThrows(GradleException.class, () -> Provisioner.target("Linux", "ppc64le"))
                .getMessage().contains("Linux/ppc64le"));
    }

    @Test
    void verifiesChecksums() throws Exception {
        Path file = tmp.resolve("a.tar.gz");
        Files.writeString(file, "archive");
        String good = sha256(Files.readAllBytes(file));
        Provisioner.verify(file, good + "  a.tar.gz\n");
        Provisioner.verify(file, good.toUpperCase() + "\n");

        Files.writeString(file, "archivE");
        assertTrue(assertThrows(GradleException.class, () -> Provisioner.verify(file, good + "  a.tar.gz\n"))
                .getMessage().contains("checksum mismatch"));

        for (String bad : new String[] {"", "\n", good.substring(1) + "  a.tar.gz", "z" + good.substring(1), "<html>Not Found</html>"}) {
            assertTrue(assertThrows(GradleException.class, () -> Provisioner.verify(file, bad))
                    .getMessage().contains("malformed checksum"), bad);
        }
    }

    @Test
    void extractsOnlyTheBinary() throws Exception {
        Path tgz = tmp.resolve("r.tar.gz");
        Files.write(tgz, tarGz(
                "groovyfmt-v1.0.0-t/README.md", '0', "readme",
                "groovyfmt-v1.0.0-t/groovyfmt", '0', "BINARY"));
        Path out = tmp.resolve("out");
        Provisioner.extract(tgz, "groovyfmt-v1.0.0-t/groovyfmt", out);
        assertEquals("BINARY", Files.readString(out));
    }

    @Test
    void ignoresPathTraversalEntries() throws Exception {
        Path dir = Files.createDirectories(tmp.resolve("a/b"));
        Path tgz = tmp.resolve("r.tar.gz");
        Files.write(tgz, tarGz(
                "../evil", '0', "pwned",
                "groovyfmt-v1.0.0-t/../../evil2", '0', "pwned",
                tmp.resolve("evil3").toString(), '0', "pwned",
                "groovyfmt-v1.0.0-t/groovyfmt", '0', "BINARY"));
        Path out = dir.resolve("groovyfmt");
        Provisioner.extract(tgz, "groovyfmt-v1.0.0-t/groovyfmt", out);
        assertEquals("BINARY", Files.readString(out));
        assertFalse(Files.exists(dir.resolve("../evil")));
        assertFalse(Files.exists(tmp.resolve("evil2")));
        assertFalse(Files.exists(tmp.resolve("evil3")));
        try (var files = Files.list(dir)) {
            assertEquals(1, files.count());
        }

        // A traversal name is never taken for the binary.
        Files.write(tgz, tarGz("groovyfmt-v1.0.0-t/../groovyfmt-v1.0.0-t/groovyfmt", '0', "x"));
        assertTrue(assertThrows(GradleException.class, () -> Provisioner.extract(tgz, "groovyfmt-v1.0.0-t/groovyfmt", tmp.resolve("o2")))
                .getMessage().contains("has no groovyfmt-v1.0.0-t/groovyfmt"));
    }

    @Test
    void rejectsALinkInPlaceOfTheBinary() throws Exception {
        Path tgz = tmp.resolve("r.tar.gz");
        Files.write(tgz, tarGz("groovyfmt-v1.0.0-t/groovyfmt", '2', ""));
        assertTrue(assertThrows(GradleException.class, () -> Provisioner.extract(tgz, "groovyfmt-v1.0.0-t/groovyfmt", tmp.resolve("o")))
                .getMessage().contains("not a regular file"));
        assertFalse(Files.exists(tmp.resolve("o")));
    }

    @Test
    void findsAssetApiUrls() {
        String json = "{\"url\":\"https://api.github.com/repos/o/r/releases/1\",\"assets\":["
                + "{\"url\":\"https://api.github.com/repos/o/r/releases/assets/11\",\"id\":11,\"node_id\":\"X\",\"name\":\"a.tar.gz\",\"uploader\":{\"url\":\"https://api.github.com/users/u\"}},"
                + "{\"url\":\"https://api.github.com/repos/o/r/releases/assets/12\",\"id\":12,\"node_id\":\"Y\",\"name\":\"a.tar.gz.sha256\",\"uploader\":{\"url\":\"https://api.github.com/users/u\"}}]}";
        assertEquals("https://api.github.com/repos/o/r/releases/assets/11", Provisioner.assetApiUrl(json, "a.tar.gz"));
        assertEquals("https://api.github.com/repos/o/r/releases/assets/12", Provisioner.assetApiUrl(json, "a.tar.gz.sha256"));
        assertThrows(GradleException.class, () -> Provisioner.assetApiUrl(json, "b.tar.gz"));
    }

    static String sha256(byte[] data) throws Exception {
        StringBuilder s = new StringBuilder();
        for (byte b : MessageDigest.getInstance("SHA-256").digest(data)) s.append(String.format("%02x", b));
        return s.toString();
    }

    /** A .tar.gz of (name, typeflag, content) triples, in ustar format. */
    static byte[] tarGz(Object... entries) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (GZIPOutputStream out = new GZIPOutputStream(bytes)) {
            for (int i = 0; i < entries.length; i += 3) {
                byte[] data = ((String) entries[i + 2]).getBytes(StandardCharsets.UTF_8);
                byte[] h = new byte[512];
                put(h, 0, (String) entries[i]);
                put(h, 100, "0000755");
                put(h, 108, "0000000");
                put(h, 116, "0000000");
                put(h, 124, String.format("%011o", data.length));
                put(h, 136, "00000000000");
                h[156] = (byte) (char) entries[i + 1];
                put(h, 257, "ustar");
                put(h, 263, "00");
                for (int j = 148; j < 156; j++) h[j] = ' ';
                int sum = 0;
                for (byte b : h) sum += b & 0xff;
                put(h, 148, String.format("%06o", sum));
                out.write(h);
                out.write(data);
                out.write(new byte[(512 - data.length % 512) % 512]);
            }
            out.write(new byte[1024]);
        }
        return bytes.toByteArray();
    }

    private static void put(byte[] h, int off, String s) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        System.arraycopy(b, 0, h, off, b.length);
    }
}
