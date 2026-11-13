// Finding, checking and downloading the groovyfmt binary. Nothing here imports vscode, so the
// unit tests run it in plain Node.
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { accessSync, constants, mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { delimiter, isAbsolute, join, resolve } from 'node:path';
import { gunzipSync, inflateRawSync } from 'node:zlib';

/** The first release with `--lsp`. */
export const MIN_LSP_VERSION = '0.9.0';
export const RELEASES = 'https://github.com/costajohnt/groovyfmt/releases/download';

/** The release asset target for each platform, as in npm/build.mjs and action.yml. */
export const TARGETS: Record<string, string> = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-arm64': 'aarch64-unknown-linux-musl',
  'linux-x64': 'x86_64-unknown-linux-musl',
  'win32-x64': 'x86_64-pc-windows-gnu',
};

export const exeName = (platform: string): string => (platform === 'win32' ? 'groovyfmt.exe' : 'groovyfmt');

export type Source = 'setting' | 'node_modules' | 'PATH' | 'download';
export interface Found {
  path: string;
  source: Source;
}

export interface Where {
  /** groovyfmt.path from user settings and from workspace or folder settings. */
  setting: { user?: string; workspace?: string };
  trusted: boolean;
  /** Workspace folder paths, first one first. */
  folders: string[];
  envPath: string;
  platform: string;
  arch: string;
}

const isExecutable = (p: string, platform: string): boolean => {
  try {
    if (!statSync(p).isFile()) return false;
    if (platform !== 'win32') accessSync(p, constants.X_OK);
    return true;
  } catch {
    return false;
  }
};

/** What `WorkspaceConfiguration.inspect` returns, as far as findBinary cares. */
export interface Inspected {
  globalValue?: string;
  workspaceValue?: string;
  workspaceFolderValue?: string;
}

/**
 * groovyfmt.path by origin: user settings, and the workspace's (the first folder's own settings
 * over the workspace's, so in a multi-root workspace the first folder decides).
 */
export const settingValues = (i: Inspected | undefined): Where['setting'] => ({
  user: i?.globalValue,
  workspace: i?.workspaceFolderValue || i?.workspaceValue,
});

/** No binary to run, and why; the caller reports it and offers no download. */
export interface Problem {
  problem: string;
}

/**
 * The binary to run, in order: groovyfmt.path, the workspace's node_modules, PATH. Workspace
 * settings and node_modules count only in a trusted workspace. `undefined` when none applies
 * (the caller may then offer a download). A groovyfmt.path that names no file is still returned,
 * so the error names the path the user set.
 *
 * A relative path is resolved against the first workspace folder only when it comes from the
 * workspace's own settings in a trusted workspace. A relative path in user settings is refused:
 * it would resolve into whatever workspace is open, trusted or not.
 */
export function findBinary(w: Where): Found | Problem | undefined {
  const workspace = w.trusted ? w.setting.workspace?.trim() : undefined;
  if (workspace) {
    const base = w.folders[0];
    if (isAbsolute(workspace)) return { path: workspace, source: 'setting' };
    if (!base) return { problem: `groovyfmt.path "${workspace}" is relative and there is no workspace folder to resolve it against.` };
    return { path: resolve(base, workspace), source: 'setting' };
  }
  const user = w.setting.user?.trim();
  if (user) {
    if (isAbsolute(user)) return { path: user, source: 'setting' };
    return { problem: `groovyfmt.path in user settings must be an absolute path, and "${user}" is not. Set the full path to the binary.` };
  }
  const exe = exeName(w.platform);
  if (w.trusted) {
    for (const dir of w.folders) {
      const nm = join(dir, 'node_modules');
      // The npm package's native binary, run directly: no Node and no .cmd shim needed. Then its
      // launcher, which is a Node script.
      // ponytail: no .bin/groovyfmt.cmd on Windows (spawning it needs a shell); the native
      // binary is there whenever the package installed properly.
      const candidates = [join(nm, '@groovyfmt', `${w.platform}-${w.arch}`, 'bin', exe)];
      if (w.platform !== 'win32') candidates.push(join(nm, '.bin', 'groovyfmt'));
      const hit = candidates.find((c) => isExecutable(c, w.platform));
      if (hit) return { path: hit, source: 'node_modules' };
    }
  }
  for (const dir of w.envPath.split(delimiter)) {
    // a relative PATH entry would resolve inside the workspace
    if (!dir || !isAbsolute(dir)) continue;
    const p = join(dir, exe);
    if (isExecutable(p, w.platform)) return { path: p, source: 'PATH' };
  }
  return undefined;
}

/** [major, minor, patch] from `groovyfmt --version` output, such as "groovyfmt 0.12.0". */
export function parseVersion(out: string): number[] | undefined {
  const m = /^groovyfmt (\d+)\.(\d+)\.(\d+)/.exec(out.trim());
  return m ? m.slice(1, 4).map(Number) : undefined;
}

export function isOlder(a: number[], b: number[]): boolean {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i];
  return false;
}

/** The binary's `--version` output; rejects if it can't be run. */
export function runVersion(path: string): Promise<string> {
  return new Promise((ok, fail) =>
    execFile(path, ['--version'], { timeout: 10_000 }, (err, stdout) => (err ? fail(err) : ok(stdout))),
  );
}

export const sha256 = (b: Buffer): string => createHash('sha256').update(b).digest('hex');

/** Throws unless `bytes` hash to `expected`, a SHA-256 digest in hex. */
export function verifySha256(bytes: Buffer, expected: string, name: string): void {
  if (!/^[0-9a-f]{64}$/.test(expected)) throw new Error(`no SHA-256 digest for ${name}`);
  const actual = sha256(bytes);
  if (actual !== expected) throw new Error(`checksum mismatch for ${name}: expected ${expected}, got ${actual}`);
}

/** The release archive's file name, such as groovyfmt-v0.12.0-x86_64-unknown-linux-musl.tar.gz. */
export function archiveName(version: string, platform: string, arch: string): string | undefined {
  const target = TARGETS[`${platform}-${arch}`];
  return target && `groovyfmt-v${version}-${target}.${platform === 'win32' ? 'zip' : 'tar.gz'}`;
}

/**
 * The checksums a release build of the extension carries (dist/checksums.json), written at
 * package time from the release archives built in the same workflow run.
 */
export interface Checksums {
  version: string;
  /** SHA-256 of each platform's release archive, by `<platform>-<arch>`. */
  sha256: Record<string, string>;
}

/**
 * Checksums for every platform's archive in `dir`, hashed from the archives themselves. Each
 * archive's `.sha256` must agree, as a check that `dir` holds what the build job produced.
 */
export function checksumsOf(version: string, dir: string): Checksums {
  const out: Checksums = { version, sha256: {} };
  for (const key of Object.keys(TARGETS)) {
    const [platform, arch] = key.split('-');
    const name = archiveName(version, platform, arch) as string;
    const digest = sha256(readFileSync(join(dir, name)));
    const listed = readFileSync(join(dir, `${name}.sha256`), 'utf8').trim().split(/\s/)[0];
    if (listed !== digest) throw new Error(`${name}.sha256 says ${listed}, the archive hashes to ${digest}`);
    out.sha256[key] = digest;
  }
  return out;
}

/** The embedded checksum for this version and platform, if the extension carries one. */
export function embeddedSha256(c: Checksums | undefined, version: string, platform: string, arch: string): string | undefined {
  return c?.version === version ? c.sha256[`${platform}-${arch}`] : undefined;
}

/** The contents of the regular file `name` in a .tar.gz. Nothing in the archive becomes a path. */
export function untarOne(tgz: Buffer, name: string): Buffer {
  const tar = gunzipSync(tgz);
  for (let p = 0; p + 512 <= tar.length; ) {
    const h = tar.subarray(p, p + 512);
    if (h.every((b) => b === 0)) break;
    const field = (a: number, b: number) => h.toString('utf8', a, b).split('\0')[0];
    const prefix = field(345, 500);
    const path = (prefix ? `${prefix}/` : '') + field(0, 100);
    const size = parseInt(field(124, 136).trim() || '0', 8);
    const type = String.fromCharCode(h[156]);
    p += 512;
    if (path === name && (type === '0' || type === '\0')) return Buffer.from(tar.subarray(p, p + size));
    // ponytail: pax and GNU long-name headers are skipped as ordinary entries; the release
    // archives' names are short, so the plain ustar name is always the real one
    p += Math.ceil(size / 512) * 512;
  }
  throw new Error(`no ${name} in the archive`);
}

// The contents of the file `name` in a zip archive. Copied from npm/build.mjs: only that entry
// is read and nothing in the archive becomes a path on disk.
// ponytail: no zip64; the release zip is a few MB. A zip64 archive fails here, loudly.
export function unzipOne(zip: Buffer, name: string): Buffer {
  const eocd = zip.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (eocd < 0) throw new Error('not a zip archive');
  const count = zip.readUInt16LE(eocd + 10);
  let p = zip.readUInt32LE(eocd + 16);
  for (let i = 0; i < count; i++) {
    if (zip.readUInt32LE(p) !== 0x02014b50) throw new Error('corrupt zip central directory');
    const method = zip.readUInt16LE(p + 10);
    const size = zip.readUInt32LE(p + 20);
    const [nameLen, extraLen, commentLen] = [28, 30, 32].map((o) => zip.readUInt16LE(p + o));
    const local = zip.readUInt32LE(p + 42);
    if (zip.toString('utf8', p + 46, p + 46 + nameLen) === name) {
      const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
      const data = zip.subarray(start, start + size);
      if (method === 0) return Buffer.from(data);
      if (method === 8) return inflateRawSync(data);
      throw new Error(`unsupported zip compression method ${method}`);
    }
    p += 46 + nameLen + extraLen + commentLen;
  }
  throw new Error(`no ${name} in the archive`);
}

/** Where a downloaded binary for this version and platform lives under `storage`. */
export const downloadPath = (storage: string, version: string, platform: string, arch: string): string =>
  join(storage, version, `${platform}-${arch}`, exeName(platform));

export interface DownloadOptions {
  /** The archive's SHA-256, from the checksums the extension carries. Required. */
  sha256?: string;
  timeoutMs?: number;
  maxBytes?: number;
}

/** The release archives are a few MB; anything this large is not one. */
export const MAX_ARCHIVE_BYTES = 64 * 1024 * 1024;

/** `url`'s body, failing past `timeoutMs` or `maxBytes`. */
async function fetchCapped(url: string, timeoutMs: number, maxBytes: number): Promise<Buffer> {
  // ponytail: redirects are followed to any host (GitHub serves release assets from another
  // one); the bytes are trusted only after they match the embedded checksum
  const res = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  if (Number(res.headers.get('content-length')) > maxBytes) throw new Error(`${url} is larger than ${maxBytes} bytes`);
  const chunks: Uint8Array[] = [];
  let size = 0;
  for await (const chunk of res.body ?? []) {
    size += chunk.length;
    if (size > maxBytes) throw new Error(`${url} is larger than ${maxBytes} bytes`);
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

/**
 * Downloads the release archive for `version`, checks it against `opts.sha256` (the checksum the
 * extension carries, never one fetched with the archive), and writes the binary to `dest`.
 * Nothing is written unless the checksum matches.
 */
export async function download(
  version: string,
  platform: string,
  arch: string,
  dest: string,
  base = RELEASES,
  opts: DownloadOptions = {},
): Promise<void> {
  const archive = archiveName(version, platform, arch);
  if (!archive) throw new Error(`groovyfmt has no prebuilt binary for ${platform}-${arch}`);
  if (!opts.sha256) throw new Error(`no checksum for ${archive} in this build of the extension`);
  const bytes = await fetchCapped(`${base}/v${version}/${archive}`, opts.timeoutMs ?? 120_000, opts.maxBytes ?? MAX_ARCHIVE_BYTES);
  verifySha256(bytes, opts.sha256, archive);
  const stem = archive.replace(/\.(zip|tar\.gz)$/, '');
  const exe = exeName(platform);
  const bin = platform === 'win32' ? unzipOne(bytes, `${stem}/${exe}`) : untarOne(bytes, `${stem}/${exe}`);
  mkdirSync(resolve(dest, '..'), { recursive: true });
  // written whole, then renamed, so a half-written binary is never at `dest`
  const tmp = `${dest}.${process.pid}.${Date.now()}.tmp`;
  try {
    writeFileSync(tmp, bin, { mode: 0o755 });
    renameSync(tmp, dest);
  } catch (e) {
    rmSync(tmp, { force: true });
    // Another window may have put the same binary there first (on Windows a running .exe
    // can't be replaced). The same bytes as the verified archive's binary are as good.
    if (!sameFile(dest, bin)) throw e;
  }
}

const sameFile = (path: string, bytes: Buffer): boolean => {
  try {
    return readFileSync(path).equals(bytes);
  } catch {
    return false;
  }
};
