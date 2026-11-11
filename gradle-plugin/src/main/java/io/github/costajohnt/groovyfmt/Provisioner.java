package io.github.costajohnt.groovyfmt;

import java.io.BufferedInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.ProxySelector;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.Comparator;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.stream.Stream;
import java.util.zip.GZIPInputStream;
import org.gradle.api.GradleException;
import org.gradle.api.logging.Logger;

/** Finds, downloads, verifies and caches the groovyfmt release binary. Mirrors action.yml. */
final class Provisioner {
    static final String REPO = "costajohnt/groovyfmt";
    private static final Pattern VERSION = Pattern.compile("v?(\\d+\\.\\d+\\.\\d+(-[0-9A-Za-z.]+)?)");

    private Provisioner() {}

    static String pluginVersion() {
        try (InputStream in = Provisioner.class.getResourceAsStream("version.txt")) {
            return new String(in.readAllBytes(), StandardCharsets.UTF_8).trim();
        } catch (IOException | NullPointerException e) {
            throw new IllegalStateException("groovyfmt plugin: version.txt is missing from the plugin jar", e);
        }
    }

    /** The release target for a JVM's {@code os.name} and {@code os.arch}. */
    static String target(String osName, String osArch) {
        String os = osName.toLowerCase(Locale.ROOT);
        String arch;
        switch (osArch.toLowerCase(Locale.ROOT)) {
            case "amd64": case "x86_64": arch = "x86_64"; break;
            case "aarch64": case "arm64": arch = "aarch64"; break;
            default: arch = null;
        }
        if (os.startsWith("windows")) {
            throw new GradleException("groovyfmt has no Windows release binary yet. Set groovyfmt.binary to a groovyfmt you built, or run it under WSL.");
        }
        String suffix = os.equals("linux") ? "unknown-linux-musl" : os.startsWith("mac") ? "apple-darwin" : null;
        if (arch == null || suffix == null) {
            throw new GradleException("groovyfmt has no release binary for " + osName + "/" + osArch
                    + " (Linux and macOS, x86_64 and arm64 only). Set groovyfmt.binary to a groovyfmt you built.");
        }
        return arch + "-" + suffix;
    }

    /** Returns the cached binary, downloading and verifying it first if needed. */
    static Path provision(String version, Path cacheRoot, String baseUrl, String token, boolean offline, Logger logger) throws IOException {
        Matcher m = VERSION.matcher(version);
        if (!m.matches()) {
            throw new GradleException("groovyfmt.version must be like 0.8.0 or v0.8.0, not '" + version + "'");
        }
        String tag = "v" + m.group(1);
        String target = target(System.getProperty("os.name"), System.getProperty("os.arch"));
        Path dir = cacheRoot.resolve(m.group(1)).resolve(target);
        Path exe = dir.resolve("groovyfmt");
        if (Files.isExecutable(exe)) return exe;
        if (offline) {
            throw new GradleException("groovyfmt " + tag + " for " + target + " is not in the cache (" + dir
                    + ") and Gradle is offline. Run once without --offline, or set groovyfmt.binary.");
        }
        String archive = "groovyfmt-" + tag + "-" + target + ".tar.gz";
        logger.lifecycle("Downloading groovyfmt {} for {}", tag, target);
        Files.createDirectories(dir);
        Path tmp = Files.createTempDirectory(dir, "download-");
        try {
            Path tgz = tmp.resolve(archive);
            Path sha = tmp.resolve(archive + ".sha256");
            if (baseUrl != null) {
                // Tests: a local "release", <base>/<tag>/<asset>. The token is never sent here.
                String base = baseUrl.endsWith("/") ? baseUrl : baseUrl + "/";
                fetch(URI.create(base + tag + "/" + archive), null, null, tgz);
                fetch(URI.create(base + tag + "/" + archive + ".sha256"), null, null, sha);
            } else if (token != null) {
                // Works for private repositories too: the API asset endpoint redirects to a signed URL.
                Path release = tmp.resolve("release.json");
                fetch(URI.create("https://api.github.com/repos/" + REPO + "/releases/tags/" + tag), token, "application/vnd.github+json", release);
                String json = Files.readString(release);
                fetch(URI.create(assetApiUrl(json, archive)), token, "application/octet-stream", tgz);
                fetch(URI.create(assetApiUrl(json, archive + ".sha256")), token, "application/octet-stream", sha);
            } else {
                String base = "https://github.com/" + REPO + "/releases/download/" + tag + "/";
                fetch(URI.create(base + archive), null, null, tgz);
                fetch(URI.create(base + archive + ".sha256"), null, null, sha);
            }
            verify(tgz, Files.readString(sha, StandardCharsets.UTF_8));
            Path extracted = tmp.resolve("groovyfmt");
            extract(tgz, "groovyfmt-" + tag + "-" + target + "/groovyfmt", extracted);
            if (!extracted.toFile().setExecutable(true)) {
                throw new IOException("could not make " + extracted + " executable");
            }
            // ponytail: no lock; concurrent builds each write a verified copy and the atomic move picks one.
            Files.move(extracted, exe, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            return exe;
        } finally {
            try (Stream<Path> files = Files.walk(tmp)) {
                files.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    /** The API URL of a release asset, from the JSON of {@code GET /repos/{repo}/releases/tags/{tag}}. */
    // ponytail: a regex instead of a JSON parser; relies on GitHub listing an asset's url before its name with no
    // nested object in between (true of the API for years). A change fails loudly here, never silently.
    static String assetApiUrl(String releaseJson, String assetName) {
        Matcher m = Pattern.compile("\"url\"\\s*:\\s*\"(https://api\\.github\\.com/repos/[^\"]+/releases/assets/\\d+)\"[^{}]*?\"name\"\\s*:\\s*\""
                + Pattern.quote(assetName) + "\"").matcher(releaseJson);
        if (!m.find()) {
            throw new GradleException("groovyfmt: the release has no asset named " + assetName);
        }
        return m.group(1);
    }

    /** Checks a file against a shasum-format line ({@code <64 hex digits>  <name>}). */
    static void verify(Path file, String shaFile) throws IOException {
        String expected = shaFile.trim().split("\\s+", 2)[0];
        if (!expected.matches("[0-9a-fA-F]{64}")) {
            throw new GradleException("groovyfmt: malformed checksum file for " + file.getFileName());
        }
        MessageDigest digest;
        try {
            digest = MessageDigest.getInstance("SHA-256");
        } catch (java.security.NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
        try (InputStream in = Files.newInputStream(file)) {
            byte[] buf = new byte[65536];
            for (int n; (n = in.read(buf)) > 0; ) digest.update(buf, 0, n);
        }
        StringBuilder actual = new StringBuilder();
        for (byte b : digest.digest()) actual.append(String.format("%02x", b));
        if (!actual.toString().equalsIgnoreCase(expected)) {
            throw new GradleException("groovyfmt: checksum mismatch for " + file.getFileName() + ": expected " + expected.toLowerCase(Locale.ROOT) + ", got " + actual);
        }
    }

    /**
     * Copies the one regular-file entry named {@code entry} out of a .tar.gz to {@code dest}. Every other entry is
     * skipped unread, so no entry name ever becomes a path on disk and traversal ({@code ../}, absolute names,
     * links) cannot happen. ustar and GNU headers; pax and GNU long-name records are skipped like any other entry.
     */
    static void extract(Path tarGz, String entry, Path dest) throws IOException {
        try (InputStream in = new GZIPInputStream(new BufferedInputStream(Files.newInputStream(tarGz)))) {
            byte[] h = new byte[512];
            while (in.readNBytes(h, 0, 512) == 512) {
                if (isZero(h)) break;
                String name = field(h, 0, 100);
                String prefix = field(h, 257, 5).equals("ustar") ? field(h, 345, 155) : "";
                String full = prefix.isEmpty() ? name : prefix + "/" + name;
                long size = octal(h, 124, 12);
                byte type = h[156];
                if (full.equals(entry)) {
                    if (type != '0' && type != 0) {
                        throw new GradleException("groovyfmt: " + entry + " in " + tarGz.getFileName() + " is not a regular file");
                    }
                    try (OutputStream out = Files.newOutputStream(dest)) {
                        copy(in, out, size);
                    }
                    return;
                }
                copy(in, OutputStream.nullOutputStream(), (size + 511) / 512 * 512);
            }
        }
        throw new GradleException("groovyfmt: " + tarGz.getFileName() + " has no " + entry);
    }

    private static String field(byte[] h, int off, int len) {
        int end = off;
        while (end < off + len && h[end] != 0) end++;
        return new String(h, off, end - off, StandardCharsets.UTF_8);
    }

    private static long octal(byte[] h, int off, int len) {
        String s = field(h, off, len).trim();
        if (!s.matches("[0-7]+")) throw new GradleException("groovyfmt: corrupt tar header (size '" + s + "')");
        return Long.parseLong(s, 8);
    }

    private static boolean isZero(byte[] h) {
        for (byte b : h) if (b != 0) return false;
        return true;
    }

    private static void copy(InputStream in, OutputStream out, long n) throws IOException {
        byte[] buf = new byte[65536];
        while (n > 0) {
            int r = in.read(buf, 0, (int) Math.min(buf.length, n));
            if (r < 0) throw new GradleException("groovyfmt: truncated archive");
            out.write(buf, 0, r);
            n -= r;
        }
    }

    /**
     * Downloads {@code uri} to {@code dest}. Redirects are followed by hand so the token goes only to the host it was
     * meant for (api.github.com), never to the storage host a release asset redirects to, and never in a URL.
     */
    private static void fetch(URI uri, String token, String accept, Path dest) throws IOException {
        if (!uri.getScheme().startsWith("http")) {
            try (InputStream in = uri.toURL().openStream()) {
                Files.copy(in, dest, StandardCopyOption.REPLACE_EXISTING);
            }
            return;
        }
        HttpClient client = HttpClient.newBuilder()
                .followRedirects(HttpClient.Redirect.NEVER)
                .proxy(ProxySelector.getDefault())
                .connectTimeout(Duration.ofSeconds(30))
                .build();
        String tokenHost = uri.getHost();
        URI current = uri;
        for (int hop = 0; hop < 10; hop++) {
            HttpRequest.Builder req = HttpRequest.newBuilder(current).timeout(Duration.ofMinutes(5)).header("User-Agent", "groovyfmt-gradle-plugin");
            if (accept != null) req.header("Accept", accept);
            if (token != null && "https".equals(current.getScheme()) && tokenHost.equals(current.getHost())) {
                req.header("Authorization", "Bearer " + token);
            }
            HttpResponse<Path> resp;
            try {
                resp = client.send(req.build(), HttpResponse.BodyHandlers.ofFile(dest));
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new IOException("interrupted", e);
            }
            int status = resp.statusCode();
            if (status >= 300 && status < 400) {
                String location = resp.headers().firstValue("Location")
                        .orElseThrow(() -> new IOException("redirect without Location from " + stripQuery(resp.uri())));
                current = current.resolve(location);
                continue;
            }
            if (status != 200) {
                String hint = status == 404 || status == 401 || status == 403
                        ? (token == null ? ". While the repository is private, set GROOVYFMT_GITHUB_TOKEN (or GITHUB_TOKEN) to a token that can read it." : ". Check that the token can read " + REPO + ".")
                        : "";
                throw new GradleException("groovyfmt: downloading " + stripQuery(current) + " failed with HTTP " + status + hint);
            }
            return;
        }
        throw new GradleException("groovyfmt: too many redirects downloading " + stripQuery(uri));
    }

    // Signed storage URLs carry credentials in the query string; keep them out of logs.
    private static String stripQuery(URI uri) {
        return uri.getScheme() + "://" + uri.getHost() + uri.getPath();
    }
}
