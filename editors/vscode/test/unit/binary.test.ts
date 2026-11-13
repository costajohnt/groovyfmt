import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import { after, before, describe, test } from 'node:test';
import { deflateRawSync } from 'node:zlib';
import {
  download,
  downloadPath,
  findBinary,
  isOlder,
  parseVersion,
  unzipOne,
  untarOne,
  verifySha256,
  type Where,
} from '../../src/binary';

const unix = process.platform !== 'win32';
const tmp = mkdtempSync(join(tmpdir(), 'groovyfmt-vscode-'));
after(() => rmSync(tmp, { recursive: true, force: true }));

const fakeBin = (path: string) => {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, '#!/bin/sh\necho groovyfmt 0.12.0\n', { mode: 0o755 });
  return path;
};
const sha = (b: Buffer) => createHash('sha256').update(b).digest('hex');

// A zip with one entry per [name, contents, deflate?], as small as unzipOne needs.
function makeZip(entries: [string, Buffer, boolean][]): Buffer {
  const locals: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const [name, data, deflate] of entries) {
    const body = deflate ? deflateRawSync(data) : data;
    const n = Buffer.from(name);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(deflate ? 8 : 0, 8);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(n.length, 26);
    const cd = Buffer.alloc(46);
    cd.writeUInt32LE(0x02014b50, 0);
    cd.writeUInt16LE(deflate ? 8 : 0, 10);
    cd.writeUInt32LE(body.length, 20);
    cd.writeUInt32LE(data.length, 24);
    cd.writeUInt16LE(n.length, 28);
    cd.writeUInt32LE(offset, 42);
    locals.push(local, n, body);
    central.push(cd, n);
    offset += 30 + n.length + body.length;
  }
  const cdBytes = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(cdBytes.length, 12);
  eocd.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, cdBytes, eocd]);
}

// A .tar.gz made by tar(1), laid out as the release archives are: <stem>/groovyfmt and friends.
function makeTarGz(stem: string, bin: Buffer): Buffer {
  const dir = mkdtempSync(join(tmp, 'tar-'));
  mkdirSync(join(dir, stem));
  writeFileSync(join(dir, stem, 'README.md'), 'readme');
  writeFileSync(join(dir, stem, 'groovyfmt'), bin, { mode: 0o755 });
  execFileSync('tar', ['-czf', join(dir, 'a.tar.gz'), '-C', dir, stem]);
  return readFileSync(join(dir, 'a.tar.gz'));
}

describe('findBinary', { skip: !unix && 'uses Unix executables' }, () => {
  const ws = join(tmp, 'ws');
  const pathDir = join(tmp, 'pathbin');
  const native = join(ws, 'node_modules', '@groovyfmt', `${process.platform}-${process.arch}`, 'bin', 'groovyfmt');
  const launcher = join(ws, 'node_modules', '.bin', 'groovyfmt');
  before(() => {
    fakeBin(native);
    fakeBin(launcher);
    fakeBin(join(pathDir, 'groovyfmt'));
    // in a relative PATH entry, which must never count
    fakeBin(join(ws, 'rel', 'groovyfmt'));
  });
  const where = (over: Partial<Where>): Where => ({
    setting: {},
    trusted: true,
    folders: [ws],
    envPath: ['rel', pathDir].join(delimiter),
    platform: process.platform,
    arch: process.arch,
    ...over,
  });

  test('groovyfmt.path comes first, workspace value over user value', () => {
    assert.deepEqual(findBinary(where({ setting: { user: '/u/groovyfmt', workspace: '/w/groovyfmt' } })), {
      path: '/w/groovyfmt',
      source: 'setting',
    });
    assert.equal(findBinary(where({ setting: { user: '/u/groovyfmt' } }))?.path, '/u/groovyfmt');
  });
  test('a relative groovyfmt.path is resolved against the first folder', () => {
    assert.equal(findBinary(where({ setting: { workspace: 'tools/groovyfmt' } }))?.path, join(ws, 'tools', 'groovyfmt'));
  });
  test('then the npm package native binary, then its launcher', () => {
    assert.deepEqual(findBinary(where({})), { path: native, source: 'node_modules' });
    rmSync(native);
    try {
      assert.deepEqual(findBinary(where({})), { path: launcher, source: 'node_modules' });
    } finally {
      fakeBin(native);
    }
  });
  test('then PATH, skipping relative entries', () => {
    assert.deepEqual(findBinary(where({ folders: [] })), { path: join(pathDir, 'groovyfmt'), source: 'PATH' });
  });
  test('a file without the execute bit does not count', () => {
    const plain = join(tmp, 'plain');
    mkdirSync(plain, { recursive: true });
    writeFileSync(join(plain, 'groovyfmt'), 'x', { mode: 0o644 });
    assert.equal(findBinary(where({ folders: [], envPath: plain })), undefined);
  });
  test('nothing found', () => {
    assert.equal(findBinary(where({ folders: [], envPath: '' })), undefined);
  });
  test('untrusted: workspace groovyfmt.path and node_modules are ignored', () => {
    const untrusted = where({ trusted: false, setting: { workspace: '/w/groovyfmt' } });
    assert.deepEqual(findBinary(untrusted), { path: join(pathDir, 'groovyfmt'), source: 'PATH' });
    assert.equal(findBinary({ ...untrusted, setting: { user: '/u/groovyfmt', workspace: '/w/groovyfmt' } })?.path, '/u/groovyfmt');
    assert.equal(findBinary({ ...untrusted, envPath: '' }), undefined);
  });
});

test('parseVersion and isOlder', () => {
  assert.deepEqual(parseVersion('groovyfmt 0.12.0\n'), [0, 12, 0]);
  assert.deepEqual(parseVersion('groovyfmt 1.2.3-rc.1'), [1, 2, 3]);
  assert.equal(parseVersion('ruff 0.1.0'), undefined);
  assert.equal(parseVersion(''), undefined);
  assert.equal(isOlder([0, 8, 0], [0, 9, 0]), true);
  assert.equal(isOlder([0, 9, 0], [0, 9, 0]), false);
  assert.equal(isOlder([0, 10, 0], [0, 9, 0]), false);
  assert.equal(isOlder([1, 0, 0], [0, 9, 9]), false);
});

test('verifySha256', () => {
  const b = Buffer.from('binary');
  verifySha256(b, `${sha(b)}  groovyfmt.tar.gz\n`, 'a');
  verifySha256(b, sha(b).toUpperCase(), 'a');
  assert.throws(() => verifySha256(b, `${'0'.repeat(64)}  x`, 'a'), /checksum mismatch for a/);
  assert.throws(() => verifySha256(b, '', 'a'), /no SHA-256 digest/);
  assert.throws(() => verifySha256(b, 'abc  x', 'a'), /no SHA-256 digest/);
});

test('untarOne reads one entry of a tar(1) archive', { skip: !unix && 'needs tar' }, () => {
  const bin = Buffer.alloc(3000, 7);
  const tgz = makeTarGz('groovyfmt-v1.0.0-x', bin);
  assert.deepEqual(untarOne(tgz, 'groovyfmt-v1.0.0-x/groovyfmt'), bin);
  assert.throws(() => untarOne(tgz, 'groovyfmt-v1.0.0-x/other'), /no groovyfmt-v1.0.0-x\/other/);
  assert.throws(() => untarOne(Buffer.from('not gzip'), 'x'));
});

test('unzipOne reads stored and deflated entries', () => {
  const a = Buffer.from('stored contents');
  const b = Buffer.alloc(5000, 'z');
  const zip = makeZip([
    ['d/a', a, false],
    ['d/groovyfmt.exe', b, true],
  ]);
  assert.deepEqual(unzipOne(zip, 'd/a'), a);
  assert.deepEqual(unzipOne(zip, 'd/groovyfmt.exe'), b);
  assert.throws(() => unzipOne(zip, 'd/missing'), /no d\/missing/);
  assert.throws(() => unzipOne(Buffer.from('nope'), 'x'), /not a zip/);
});

describe('download', { skip: !unix && 'needs tar' }, () => {
  const files = new Map<string, Buffer>();
  let server: Server;
  let base = '';
  const bin = Buffer.from('#!/bin/sh\necho groovyfmt 1.0.0\n');
  const tgz = makeTarGz('groovyfmt-v1.0.0-x86_64-unknown-linux-musl', bin);
  const zipBin = Buffer.alloc(4000, 'w');
  const zip = makeZip([['groovyfmt-v1.0.0-x86_64-pc-windows-gnu/groovyfmt.exe', zipBin, true]]);
  before(async () => {
    files.set('/v1.0.0/groovyfmt-v1.0.0-x86_64-unknown-linux-musl.tar.gz', tgz);
    files.set('/v1.0.0/groovyfmt-v1.0.0-x86_64-unknown-linux-musl.tar.gz.sha256', Buffer.from(`${sha(tgz)}  x\n`));
    files.set('/v1.0.0/groovyfmt-v1.0.0-x86_64-pc-windows-gnu.zip', zip);
    files.set('/v1.0.0/groovyfmt-v1.0.0-x86_64-pc-windows-gnu.zip.sha256', Buffer.from(`${sha(zip)}  x\n`));
    // linux-arm64's checksum does not match its archive
    files.set('/v1.0.0/groovyfmt-v1.0.0-aarch64-unknown-linux-musl.tar.gz', tgz);
    files.set('/v1.0.0/groovyfmt-v1.0.0-aarch64-unknown-linux-musl.tar.gz.sha256', Buffer.from(`${'0'.repeat(64)}  x\n`));
    server = createServer((req, res) => {
      const body = files.get(req.url ?? '');
      res.writeHead(body ? 200 : 404).end(body);
    });
    await new Promise<void>((ok) => server.listen(0, '127.0.0.1', ok));
    const addr = server.address();
    base = `http://127.0.0.1:${typeof addr === 'object' && addr ? addr.port : 0}`;
  });
  after(() => server.close());

  test('writes the verified binary, executable', async () => {
    const dest = downloadPath(join(tmp, 'store'), '1.0.0', 'linux', 'x64');
    await download('1.0.0', 'linux', 'x64', dest, base);
    assert.deepEqual(readFileSync(dest), bin);
    assert.equal(statSync(dest).mode & 0o111, 0o111);
    assert.deepEqual(readdirSync(join(dest, '..')), ['groovyfmt']);
  });
  test('the Windows zip', async () => {
    const dest = downloadPath(join(tmp, 'store'), '1.0.0', 'win32', 'x64');
    await download('1.0.0', 'win32', 'x64', dest, base);
    assert.deepEqual(readFileSync(dest), zipBin);
    assert.ok(dest.endsWith('groovyfmt.exe'));
  });
  test('a checksum mismatch writes nothing', async () => {
    const dest = downloadPath(join(tmp, 'store'), '1.0.0', 'linux', 'arm64');
    await assert.rejects(download('1.0.0', 'linux', 'arm64', dest, base), /checksum mismatch/);
    assert.equal(existsSync(join(dest, '..')), false);
  });
  test('a missing release', async () => {
    await assert.rejects(download('2.0.0', 'linux', 'x64', join(tmp, 'x', 'groovyfmt'), base), /HTTP 404/);
  });
  test('a platform without a prebuilt binary', async () => {
    await assert.rejects(download('1.0.0', 'freebsd', 'x64', join(tmp, 'y', 'groovyfmt'), base), /no prebuilt binary for freebsd-x64/);
  });
});
