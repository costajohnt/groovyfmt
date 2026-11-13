// Starts VS Code with the extension and test/integration/suite.ts on a fresh workspace whose
// settings point groovyfmt.path at GROOVYFMT_BIN (a groovyfmt build, absolute path).
// VSCODE_TEST_ARGS adds launch arguments, such as --ozone-platform=headless without a display.
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { runTests } from '@vscode/test-electron';

async function main(): Promise<void> {
  const bin = process.env.GROOVYFMT_BIN;
  if (!bin || !isAbsolute(bin)) throw new Error('set GROOVYFMT_BIN to the absolute path of a groovyfmt binary');
  const ws = mkdtempSync(join(tmpdir(), 'groovyfmt-vscode-it-'));
  try {
    mkdirSync(join(ws, '.vscode'));
    writeFileSync(join(ws, '.vscode', 'settings.json'), JSON.stringify({ 'groovyfmt.path': bin }));
    writeFileSync(join(ws, 'A.groovy'), 'class A {\ndef f() {\nprintln 1;\n}\n}\n');
    await runTests({
      extensionDevelopmentPath: resolve(__dirname, '../../..'),
      extensionTestsPath: join(__dirname, 'suite'),
      launchArgs: [
        ws,
        '--disable-extensions',
        '--disable-workspace-trust',
        '--skip-welcome',
        '--skip-release-notes',
        ...(process.env.VSCODE_TEST_ARGS ?? '').split(' ').filter(Boolean),
      ],
    });
  } finally {
    rmSync(ws, { recursive: true, force: true });
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
