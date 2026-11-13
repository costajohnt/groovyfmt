// Finding, checking and downloading the groovyfmt binary. Nothing here imports vscode, so the
// unit tests run it in plain Node.
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { accessSync, constants, mkdirSync, renameSync, statSync, writeFileSync } from 'node:fs';
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

/**
 * The binary to run, in order: groovyfmt.path, the workspace's node_modules, PATH. Workspace
 * settings and node_modules count only in a trusted workspace. `undefined` when none applies
 * (the caller may then offer a download). A groovyfmt.path that names no file is still returned,
 * so the error names the path the user set.
 */
export function findBinary(w: Where): Found | undefined {
  const setting = (w.trusted ? w.setting.workspace || w.setting.user : w.setting.user)?.trim();
  if (setting) {
    const base = w.folders[0];
    return { path: isAbsolute(setting) || !base ? setting : resolve(base, setting), source: 'setting' };
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

/** Throws unless `bytes` hash to the digest at the start of the `.sha256` file's text. */
export function verifySha256(bytes: Buffer, sha256File: string, name: string): void {
  const expected = sha256File.trim().split(/\s/)[0].toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(expected)) throw new Error(`${name}.sha256 holds no SHA-256 digest`);
  const actual = createHash('sha256').update(bytes).digest('hex');
  if (actual !== expected) throw new Error(`checksum mismatch for ${name}: expected ${expected}, got ${actual}`);
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

/**
 * Downloads the release archive for `version` and its `.sha256`, checks one against the other,
 * and writes the binary to `dest`. Nothing is written unless the checksum matches.
 */
export async function download(
  version: string,
  platform: string,
  arch: string,
  dest: string,
  base = RELEASES,
): Promise<void> {
  const target = TARGETS[`${platform}-${arch}`];
  if (!target) throw new Error(`groovyfmt has no prebuilt binary for ${platform}-${arch}`);
  const tag = `v${version}`;
  const stem = `groovyfmt-${tag}-${target}`;
  const archive = `${stem}.${platform === 'win32' ? 'zip' : 'tar.gz'}`;
  const get = async (name: string) => {
    const url = `${base}/${tag}/${name}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
    return Buffer.from(await res.arrayBuffer());
  };
  const [bytes, sum] = await Promise.all([get(archive), get(`${archive}.sha256`)]);
  verifySha256(bytes, sum.toString('utf8'), archive);
  const exe = exeName(platform);
  const bin = platform === 'win32' ? unzipOne(bytes, `${stem}/${exe}`) : untarOne(bytes, `${stem}/${exe}`);
  mkdirSync(resolve(dest, '..'), { recursive: true });
  // written whole, then renamed, so a half-written binary is never at `dest`
  const tmp = `${dest}.${process.pid}.tmp`;
  writeFileSync(tmp, bin, { mode: 0o755 });
  renameSync(tmp, dest);
}
