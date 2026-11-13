// Two runs of VS Code with the extension and test/integration/suite.ts:
//
// trusted: a workspace whose settings point groovyfmt.path at GROOVYFMT_BIN (a groovyfmt
// build, absolute path); diagnostics, formatting and fix all must work.
//
// untrusted: VS Code in Restricted Mode, on a workspace that plants executables where
// groovyfmt.path (workspace settings) and node_modules point. Each writes a marker file if run;
// none may run.
//
// VSCODE_TEST_ARGS adds launch arguments, such as --ozone-platform=headless without a display.
// GROOVYFMT_IT_ONLY=trusted or untrusted runs just one.
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { downloadAndUnzipVSCode, runTests } from '@vscode/test-electron';

const extensionDevelopmentPath = resolve(__dirname, '../../..');
const extensionTestsPath = join(__dirname, 'suite');
const extra = (process.env.VSCODE_TEST_ARGS ?? '').split(' ').filter(Boolean);
const sample = 'class A {\ndef f() {\nprintln 1;\n}\n}\n';

async function trusted(ws: string): Promise<void> {
  // runTests adds --disable-workspace-trust: the workspace is trusted
  const bin = process.env.GROOVYFMT_BIN;
  if (!bin || !isAbsolute(bin)) throw new Error('set GROOVYFMT_BIN to the absolute path of a groovyfmt binary');
  mkdirSync(join(ws, '.vscode'));
  writeFileSync(join(ws, '.vscode', 'settings.json'), JSON.stringify({ 'groovyfmt.path': bin }));
  writeFileSync(join(ws, 'A.groovy'), sample);
  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    extensionTestsEnv: { GROOVYFMT_IT_MODE: 'trusted' },
    launchArgs: [ws, '--disable-extensions', ...extra],
  });
}

async function untrusted(dir: string): Promise<void> {
  const ws = join(dir, 'ws');
  const markers = join(dir, 'markers');
  mkdirSync(markers);
  const plant = (path: string, name: string) => {
    mkdirSync(join(path, '..'), { recursive: true });
    writeFileSync(path, `#!/bin/sh\necho ran > '${join(markers, name)}'\necho groovyfmt 0.12.0\n`, { mode: 0o755 });
  };
  const setting = join(ws, 'tools', 'groovyfmt');
  plant(setting, 'setting');
  plant(join(ws, 'node_modules', '@groovyfmt', `${process.platform}-${process.arch}`, 'bin', 'groovyfmt'), 'native');
  plant(join(ws, 'node_modules', '.bin', 'groovyfmt'), 'launcher');
  mkdirSync(join(ws, '.vscode'));
  writeFileSync(join(ws, '.vscode', 'settings.json'), JSON.stringify({ 'groovyfmt.path': setting }));
  writeFileSync(join(ws, 'A.groovy'), sample);
  // a fresh profile with Workspace Trust on and no startup prompt, so the folder opens restricted
  const data = join(dir, 'user-data');
  mkdirSync(join(data, 'User'), { recursive: true });
  writeFileSync(
    join(data, 'User', 'settings.json'),
    JSON.stringify({ 'security.workspace.trust.enabled': true, 'security.workspace.trust.startupPrompt': 'never' }),
  );
  // runTests always adds --disable-workspace-trust, so this run starts VS Code itself with
  // runTests' other arguments
  const exe = await downloadAndUnzipVSCode();
  const args = [
    ws,
    '--no-sandbox',
    '--disable-gpu-sandbox',
    '--disable-updates',
    '--skip-welcome',
    '--skip-release-notes',
    '--no-cached-data',
    '--disable-extensions',
    `--user-data-dir=${data}`,
    `--extensions-dir=${join(dir, 'extensions')}`,
    `--extensionTestsPath=${extensionTestsPath}`,
    `--extensionDevelopmentPath=${extensionDevelopmentPath}`,
    ...extra,
  ];
  const exit = await new Promise<number | null>((ok, fail) =>
    spawn(exe, args, {
      stdio: 'inherit',
      // no groovyfmt on PATH either, so nothing outside the workspace is found
      env: { ...process.env, PATH: '/usr/bin:/bin', GROOVYFMT_IT_MODE: 'untrusted', GROOVYFMT_IT_MARKERS: markers },
    })
      .on('error', fail)
      .on('exit', ok),
  );
  const ran = readdirSync(markers);
  if (ran.length) throw new Error(`untrusted run executed planted binaries: ${ran.join(', ')}`);
  if (exit !== 0) throw new Error(`untrusted run failed with exit code ${exit}`);
}

async function main(): Promise<void> {
  const only = process.env.GROOVYFMT_IT_ONLY;
  for (const [name, run] of [
    ['trusted', trusted],
    ['untrusted', untrusted],
  ] as const) {
    if (only && only !== name) continue;
    const dir = mkdtempSync(join(tmpdir(), `groovyfmt-vscode-it-${name}-`));
    try {
      await run(dir);
      console.log(`integration (${name}): passed`);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
