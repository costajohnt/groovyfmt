#!/usr/bin/env node
// Assembles the npm packages in npm/dist/ from a directory of release archives:
//
//   node npm/build.mjs <version> <archives-dir>
//
// <archives-dir> holds groovyfmt-v<version>-<target>.tar.gz (.zip for Windows)
// and its .sha256 for every target below (gh release download v<version>
// --pattern 'groovyfmt-*'). Each archive is checked against its .sha256 before
// use. Node stdlib and tar.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { inflateRawSync } from 'node:zlib';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const targets = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-arm64': 'aarch64-unknown-linux-musl',
  'linux-x64': 'x86_64-unknown-linux-musl',
  'win32-x64': 'x86_64-pc-windows-gnu',
};

// The contents of the file `name` in a zip archive, or an error. Only that entry is read
// and nothing in the archive becomes a path on disk.
// Sizes come from the central directory, so data descriptors are fine.
// ponytail: no zip64; the release zip is a few MB. A zip64 archive fails here, loudly.
const unzipOne = (zip, name) => {
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
};

const [rawVersion, archivesArg] = process.argv.slice(2);
if (!rawVersion || !archivesArg) {
  console.error('usage: node npm/build.mjs <version> <archives-dir>');
  process.exit(2);
}
const version = rawVersion.replace(/^v/, '');
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version)) {
  console.error(`build.mjs: not a version: ${rawVersion}`);
  process.exit(2);
}

const npmDir = dirname(fileURLToPath(import.meta.url));
const root = resolve(npmDir, '..');
const archives = resolve(archivesArg);
const dist = join(npmDir, 'dist');
rmSync(dist, { recursive: true, force: true });

const common = {
  version,
  description: 'Fast formatter and linter for Groovy, Gradle build scripts, and Jenkinsfiles',
  license: 'MIT',
  homepage: 'https://github.com/costajohnt/groovyfmt#readme',
  // npm provenance requires this to match the publishing repository exactly.
  repository: { type: 'git', url: 'git+https://github.com/costajohnt/groovyfmt.git' },
};
const write = (dir, pkg) => {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'package.json'), `${JSON.stringify(pkg, null, 2)}\n`);
  cpSync(join(root, 'LICENSE'), join(dir, 'LICENSE'));
};

const work = mkdtempSync(join(tmpdir(), 'groovyfmt-npm-'));
try {
  for (const [platform, target] of Object.entries(targets)) {
    const stem = `groovyfmt-v${version}-${target}`;
    const windows = platform.startsWith('win32');
    const exe = windows ? 'groovyfmt.exe' : 'groovyfmt';
    const archive = join(archives, `${stem}.${windows ? 'zip' : 'tar.gz'}`);
    const bytes = readFileSync(archive);
    const expected = readFileSync(`${archive}.sha256`, 'utf8').split(/\s/)[0];
    const actual = createHash('sha256').update(bytes).digest('hex');
    if (expected !== actual) {
      throw new Error(`checksum mismatch for ${archive}: expected ${expected}, got ${actual}`);
    }
    if (windows) {
      mkdirSync(join(work, stem), { recursive: true });
      writeFileSync(join(work, stem, exe), unzipOne(bytes, `${stem}/${exe}`));
    } else {
      execFileSync('tar', ['-xzf', archive, '-C', work, `${stem}/${exe}`]);
    }

    const [os, cpu] = platform.split('-');
    const dir = join(dist, platform);
    write(dir, {
      name: `@groovyfmt/${platform}`,
      ...common,
      description: `The groovyfmt binary for ${platform}; install the groovyfmt package instead`,
      os: [os],
      cpu: [cpu],
      files: ['bin'],
    });
    mkdirSync(join(dir, 'bin'));
    cpSync(join(work, stem, exe), join(dir, 'bin', exe));
    chmodSync(join(dir, 'bin', exe), 0o755);
  }

  const main = join(dist, 'groovyfmt');
  write(main, {
    name: 'groovyfmt',
    ...common,
    keywords: ['groovy', 'gradle', 'jenkinsfile', 'formatter', 'linter'],
    bin: { groovyfmt: 'bin/groovyfmt.js' },
    files: ['bin'],
    engines: { node: '>=18' },
    optionalDependencies: Object.fromEntries(Object.keys(targets).map((p) => [`@groovyfmt/${p}`, version])),
  });
  cpSync(join(npmDir, 'groovyfmt', 'bin'), join(main, 'bin'), { recursive: true });
  cpSync(join(root, 'README.md'), join(main, 'README.md'));
} finally {
  rmSync(work, { recursive: true, force: true });
}
console.log(`built ${Object.keys(targets).length + 1} packages for ${version} in ${dist}`);
