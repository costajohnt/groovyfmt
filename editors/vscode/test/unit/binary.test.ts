import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createServer, type Server, type ServerResponse } from 'node:http';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import { after, before, describe, test } from 'node:test';
import { deflateRawSync } from 'node:zlib';
import {
  TARGETS,
  archiveName,
  checksumsOf,
  download,
  downloadPath,
  embeddedSha256,
  findBinary,
  isOlder,
  parseVersion,
  settingValues,
  sha256,
  unzipOne,
  untarOne,
  verifySha256,
  type Where,
} from '../../src/binary';

const unix = process.platform !== 'win32';
const root = process.getuid?.() === 0;
const tmp = mkdtempSync(join(tmpdir(), 'groovyfmt-vscode-'));
after(() => rmSync(tmp, { recursive: true, force: true }));

const fakeBin = (path: string) => {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, '#!/bin/sh\necho groovyfmt 0.12.0\n', { mode: 0o755 });
  return path;
};

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
  const onPath = join(pathDir, 'groovyfmt');
  const native = join(ws, 'node_modules', '@groovyfmt', `${process.platform}-${process.arch}`, 'bin', 'groovyfmt');
  const launcher = join(ws, 'node_modules', '.bin', 'groovyfmt');
  // what a hostile repository would ship for a relative groovyfmt.path to land on
  const planted = join(ws, 'bin', 'groovyfmt');
  before(() => {
    fakeBin(native);
    fakeBin(launcher);
    fakeBin(onPath);
    fakeBin(planted);
    // where the relative PATH entry `rel` resolves while the cwd is the workspace
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
  const inWorkspace = <T>(f: () => T): T => {
    const cwd = process.cwd();
    process.chdir(ws);
    try {
      return f();
    } finally {
      process.chdir(cwd);
    }
  };

  test('groovyfmt.path comes first, workspace value over user value', () => {
    assert.deepEqual(findBinary(where({ setting: { user: '/u/groovyfmt', workspace: '/w/groovyfmt' } })), {
      path: '/w/groovyfmt',
      source: 'setting',
    });
    assert.deepEqual(findBinary(where({ setting: { user: '/u/groovyfmt' } })), { path: '/u/groovyfmt', source: 'setting' });
  });
  test('trusted: a relative workspace value is resolved against the first folder', () => {
    assert.deepEqual(findBinary(where({ folders: [ws, '/other'], setting: { workspace: 'bin/groovyfmt' } })), {
      path: planted,
      source: 'setting',
    });
    assert.match((findBinary(where({ folders: [], setting: { workspace: 'bin/groovyfmt' } })) as { problem: string }).problem, /no workspace folder/);
  });
  test('untrusted: a relative user value never resolves into the workspace', () => {
    for (const user of ['bin/groovyfmt', './bin/groovyfmt', 'groovyfmt']) {
      const r = findBinary(where({ trusted: false, setting: { user } }));
      assert.ok(r && 'problem' in r, `${user}: ${JSON.stringify(r)}`);
      assert.match(r.problem, /must be an absolute path/);
    }
  });
  test('trusted: a relative user value is refused too', () => {
    const r = findBinary(where({ setting: { user: 'bin/groovyfmt' } }));
    assert.ok(r && 'problem' in r, JSON.stringify(r));
  });
  test('untrusted: a workspace value of any kind is ignored', () => {
    for (const workspace of ['/w/groovyfmt', 'bin/groovyfmt', planted]) {
      assert.deepEqual(findBinary(where({ trusted: false, setting: { workspace } })), { path: onPath, source: 'PATH' });
      assert.deepEqual(findBinary(where({ trusted: false, setting: { workspace, user: '/u/groovyfmt' } })), {
        path: '/u/groovyfmt',
        source: 'setting',
      });
    }
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
  test('untrusted: node_modules is ignored', () => {
    assert.deepEqual(findBinary(where({ trusted: false })), { path: onPath, source: 'PATH' });
  });
  test('then PATH, skipping relative entries', () => {
    // from the workspace, `rel` would find the planted rel/groovyfmt
    assert.deepEqual(
      inWorkspace(() => findBinary(where({ folders: [] }))),
      { path: onPath, source: 'PATH' },
    );
    assert.equal(inWorkspace(() => findBinary(where({ folders: [], envPath: 'rel' }))), undefined);
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
});

test('settingValues maps inspect() origins', () => {
  assert.deepEqual(settingValues(undefined), { user: undefined, workspace: undefined });
  assert.deepEqual(settingValues({ globalValue: '/g' }), { user: '/g', workspace: undefined });
  assert.deepEqual(settingValues({ workspaceValue: '/w' }), { user: undefined, workspace: '/w' });
  assert.deepEqual(settingValues({ workspaceFolderValue: '/f' }), { user: undefined, workspace: '/f' });
  assert.deepEqual(settingValues({ globalValue: '/g', workspaceValue: '/w', workspaceFolderValue: '/f' }), {
    user: '/g',
    workspace: '/f',
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
  verifySha256(b, sha256(b), 'a');
  assert.throws(() => verifySha256(b, '0'.repeat(64), 'a'), /checksum mismatch for a/);
  assert.throws(() => verifySha256(b, '', 'a'), /no SHA-256 digest/);
  assert.throws(() => verifySha256(b, 'abc', 'a'), /no SHA-256 digest/);
});

test('checksumsOf hashes each archive and checks its .sha256', () => {
  const dir = mkdtempSync(join(tmp, 'sums-'));
  const expected: Record<string, string> = {};
  for (const key of Object.keys(TARGETS)) {
    const [platform, arch] = key.split('-');
    const name = archiveName('1.2.3', platform, arch) as string;
    const bytes = Buffer.from(`archive for ${key}`);
    writeFileSync(join(dir, name), bytes);
    writeFileSync(join(dir, `${name}.sha256`), `${sha256(bytes)}  ${name}\n`);
    expected[key] = sha256(bytes);
  }
  const sums = checksumsOf('1.2.3', dir);
  assert.deepEqual(sums, { version: '1.2.3', sha256: expected });
  assert.equal(embeddedSha256(sums, '1.2.3', 'linux', 'x64'), expected['linux-x64']);
  assert.equal(embeddedSha256(sums, '1.2.4', 'linux', 'x64'), undefined);
  assert.equal(embeddedSha256(undefined, '1.2.3', 'linux', 'x64'), undefined);
  assert.equal(archiveName('1.2.3', 'win32', 'x64'), 'groovyfmt-v1.2.3-x86_64-pc-windows-gnu.zip');

  writeFileSync(join(dir, `${archiveName('1.2.3', 'darwin', 'x64')}.sha256`), `${'0'.repeat(64)}  x\n`);
  assert.throws(() => checksumsOf('1.2.3', dir), /\.sha256 says 0+, the archive hashes to/);
  assert.throws(() => checksumsOf('9.9.9', dir), /ENOENT/);
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
  type Handler = (res: ServerResponse) => void;
  const files = new Map<string, Buffer | Handler>();
  let server: Server;
  let base = '';
  const linux = 'groovyfmt-v1.0.0-x86_64-unknown-linux-musl.tar.gz';
  const arm = 'groovyfmt-v1.0.0-aarch64-unknown-linux-musl.tar.gz';
  const win = 'groovyfmt-v1.0.0-x86_64-pc-windows-gnu.zip';
  const bin = Buffer.from('#!/bin/sh\necho groovyfmt 1.0.0\n');
  const tgz = makeTarGz('groovyfmt-v1.0.0-x86_64-unknown-linux-musl', bin);
  const evil = makeTarGz('groovyfmt-v1.0.0-aarch64-unknown-linux-musl', Buffer.from('#!/bin/sh\necho evil\n'));
  const zipBin = Buffer.alloc(4000, 'w');
  const zip = makeZip([['groovyfmt-v1.0.0-x86_64-pc-windows-gnu/groovyfmt.exe', zipBin, true]]);
  // what the extension carries for each archive
  const real = { linux: sha256(tgz), arm: sha256(makeTarGz('groovyfmt-v1.0.0-aarch64-unknown-linux-musl', bin)), win: sha256(zip) };
  let fetched: string[] = [];
  const serve = (name: string, body: Buffer | Handler, sum?: Buffer) => {
    files.set(`/v1.0.0/${name}`, body);
    // the release's own checksum file, which download must not rely on
    if (sum) files.set(`/v1.0.0/${name}.sha256`, Buffer.from(`${sha256(sum)}  ${name}\n`));
  };
  before(async () => {
    serve(linux, tgz, tgz);
    serve(win, zip, zip);
    // the release was rewritten: another archive, with a .sha256 that matches it
    serve(arm, evil, evil);
    serve('groovyfmt-v1.0.0-x86_64-apple-darwin.tar.gz', (res) => {
      res.writeHead(200, { 'content-length': String(100 * 1024 * 1024) });
      res.write(Buffer.alloc(1024));
    });
    serve('groovyfmt-v1.0.0-aarch64-apple-darwin.tar.gz', (res) => {
      res.writeHead(200);
      const chunk = Buffer.alloc(64 * 1024);
      const more = () => {
        if (res.destroyed) return;
        if (!res.write(chunk)) res.once('drain', more);
        else setImmediate(more);
      };
      more();
    });
    files.set('/v2.0.0/stall', () => undefined);
    server = createServer((req, res) => {
      fetched.push(req.url ?? '');
      const body = files.get(req.url ?? '');
      if (typeof body === 'function') return body(res);
      res.writeHead(body ? 200 : 404).end(body);
    });
    await new Promise<void>((ok) => server.listen(0, '127.0.0.1', ok));
    const addr = server.address();
    base = `http://127.0.0.1:${typeof addr === 'object' && addr ? addr.port : 0}`;
  });
  after(() => {
    server.closeAllConnections();
    server.close();
  });
  const store = (platform: string, arch: string) => downloadPath(mkdtempSync(join(tmp, 'store-')), '1.0.0', platform, arch);

  test('writes the binary that matches the embedded checksum, executable', async () => {
    fetched = [];
    const dest = store('linux', 'x64');
    await download('1.0.0', 'linux', 'x64', dest, base, { sha256: real.linux });
    assert.deepEqual(readFileSync(dest), bin);
    assert.equal(statSync(dest).mode & 0o111, 0o111);
    assert.deepEqual(readdirSync(join(dest, '..')), ['groovyfmt']);
    // the release's .sha256 is never fetched
    assert.deepEqual(fetched, [`/v1.0.0/${linux}`]);
  });
  test('the Windows zip', async () => {
    const dest = store('win32', 'x64');
    await download('1.0.0', 'win32', 'x64', dest, base, { sha256: real.win });
    assert.deepEqual(readFileSync(dest), zipBin);
    assert.ok(dest.endsWith('groovyfmt.exe'));
  });
  test('a swapped archive with a matching swapped .sha256 is rejected', async () => {
    const dest = store('linux', 'arm64');
    await assert.rejects(download('1.0.0', 'linux', 'arm64', dest, base, { sha256: real.arm }), /checksum mismatch/);
    assert.equal(existsSync(join(dest, '..')), false);
  });
  test('no embedded checksum: nothing is fetched', async () => {
    fetched = [];
    await assert.rejects(download('1.0.0', 'linux', 'x64', store('linux', 'x64'), base), /no checksum/);
    assert.deepEqual(fetched, []);
  });
  test('a missing release', async () => {
    await assert.rejects(download('2.0.0', 'linux', 'x64', store('linux', 'x64'), base, { sha256: real.linux }), /HTTP 404/);
  });
  test('a platform without a prebuilt binary', async () => {
    await assert.rejects(download('1.0.0', 'freebsd', 'x64', store('freebsd', 'x64'), base, { sha256: real.linux }), /no prebuilt binary for freebsd-x64/);
  });
  test('a content-length over the cap', { timeout: 10_000 }, async () => {
    await assert.rejects(
      download('1.0.0', 'darwin', 'x64', store('darwin', 'x64'), base, { sha256: real.linux, maxBytes: 1024 * 1024 }),
      /larger than/,
    );
  });
  test('a body that keeps coming past the cap', { timeout: 10_000 }, async () => {
    await assert.rejects(
      download('1.0.0', 'darwin', 'arm64', store('darwin', 'arm64'), base, { sha256: real.linux, maxBytes: 1024 * 1024 }),
      /larger than/,
    );
  });
  test('a stalled response times out', { timeout: 10_000 }, async () => {
    await assert.rejects(
      download('1.0.0', 'linux', 'x64', store('linux', 'x64'), `${base}/v2.0.0/stall#`, { sha256: real.linux, timeoutMs: 300 }),
      /abort|timeout/i,
    );
  });
  test('a failed rename leaves no temp file', async () => {
    const dest = store('linux', 'x64');
    mkdirSync(join(dest, 'occupied'), { recursive: true });
    await assert.rejects(download('1.0.0', 'linux', 'x64', dest, base, { sha256: real.linux }));
    assert.deepEqual(readdirSync(join(dest, '..')), ['groovyfmt']);
  });
  test('the same binary already in place is used when the write fails', { skip: root && 'root ignores permissions' }, async () => {
    const dest = store('linux', 'x64');
    mkdirSync(join(dest, '..'), { recursive: true });
    writeFileSync(dest, bin, { mode: 0o755 });
    chmodSync(join(dest, '..'), 0o555);
    try {
      await download('1.0.0', 'linux', 'x64', dest, base, { sha256: real.linux });
      // a different file there is not taken for it
      chmodSync(join(dest, '..'), 0o755);
      writeFileSync(dest, 'other', { mode: 0o755 });
      chmodSync(join(dest, '..'), 0o555);
      await assert.rejects(download('1.0.0', 'linux', 'x64', dest, base, { sha256: real.linux }), /EACCES/);
    } finally {
      chmodSync(join(dest, '..'), 0o755);
    }
    assert.deepEqual(readdirSync(join(dest, '..')), ['groovyfmt']);
  });
});
