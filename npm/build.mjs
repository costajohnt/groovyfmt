#!/usr/bin/env node
// Assembles the npm packages in npm/dist/ from a directory of release archives:
//
//   node npm/build.mjs <version> <archives-dir>
//
// <archives-dir> holds groovyfmt-v<version>-<target>.tar.gz and its .sha256 for
// every target below (gh release download v<version> --pattern 'groovyfmt-*').
// Each archive is checked against its .sha256 before use. Node stdlib and tar.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const targets = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-arm64': 'aarch64-unknown-linux-musl',
  'linux-x64': 'x86_64-unknown-linux-musl',
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
    const archive = join(archives, `${stem}.tar.gz`);
    const expected = readFileSync(`${archive}.sha256`, 'utf8').split(/\s/)[0];
    const actual = createHash('sha256').update(readFileSync(archive)).digest('hex');
    if (expected !== actual) {
      throw new Error(`checksum mismatch for ${archive}: expected ${expected}, got ${actual}`);
    }
    execFileSync('tar', ['-xzf', archive, '-C', work, `${stem}/groovyfmt`]);

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
    cpSync(join(work, stem, 'groovyfmt'), join(dir, 'bin', 'groovyfmt'));
    chmodSync(join(dir, 'bin', 'groovyfmt'), 0o755);
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
