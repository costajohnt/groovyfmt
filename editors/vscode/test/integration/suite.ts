// Runs inside VS Code (see run.ts): a lint diagnostic arrives, format document fixes the
// indentation, and the fix-all code action removes the semicolon.
import assert from 'node:assert/strict';
import { readdirSync } from 'node:fs';
import { join } from 'node:path';
import * as vscode from 'vscode';

function diagnostics(uri: vscode.Uri, timeoutMs: number): Promise<vscode.Diagnostic[]> {
  return new Promise((ok, fail) => {
    const done = () => vscode.languages.getDiagnostics(uri).filter((d) => d.source === 'groovyfmt');
    const timer = setTimeout(() => {
      sub.dispose();
      fail(new Error(`no groovyfmt diagnostics within ${timeoutMs} ms`));
    }, timeoutMs);
    const check = () => {
      const ds = done();
      if (ds.length === 0) return;
      clearTimeout(timer);
      sub.dispose();
      ok(ds);
    };
    const sub = vscode.languages.onDidChangeDiagnostics(check);
    check();
  });
}

export function run(): Promise<void> {
  return process.env.GROOVYFMT_IT_MODE === 'untrusted' ? untrusted() : trusted();
}

const sleep = (ms: number) => new Promise((ok) => setTimeout(ok, ms));

// Restricted Mode: the workspace's groovyfmt.path and node_modules point at planted
// executables that leave a marker in GROOVYFMT_IT_MARKERS when run. None may run, and the
// extension must report that it has no binary.
async function untrusted(): Promise<void> {
  assert.equal(vscode.workspace.isTrusted, false, 'the workspace is trusted: Restricted Mode did not take effect');
  const markers = process.env.GROOVYFMT_IT_MARKERS;
  assert.ok(markers, 'GROOVYFMT_IT_MARKERS is not set');
  const ext = vscode.extensions.getExtension<{ lastProblem(): string | undefined }>('groovyfmt.groovyfmt');
  assert.ok(ext, 'extension not found');
  const api = await ext.activate();
  const folder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(folder, 'no workspace folder');
  const uri = vscode.Uri.file(join(folder.uri.fsPath, 'A.groovy'));
  await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
  // ponytail: polled, the extension has no event for it; 30 s at most
  for (let i = 0; i < 300 && !api.lastProblem(); i++) await sleep(100);
  assert.match(api.lastProblem() ?? '', /was not found in groovyfmt\.path, node_modules or PATH/);
  // the restart a config read or trust check might trigger has time to happen
  await sleep(3000);
  assert.deepEqual(readdirSync(markers), [], 'a planted binary ran');
  assert.deepEqual(vscode.languages.getDiagnostics(uri).filter((d) => d.source === 'groovyfmt'), []);
  console.log(`integration: restricted mode ran nothing from the workspace (${api.lastProblem()})`);
}

async function trusted(): Promise<void> {
  const folder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(folder, 'no workspace folder');
  const uri = vscode.Uri.file(join(folder.uri.fsPath, 'A.groovy'));
  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(doc);
  assert.equal(doc.languageId, 'groovy');

  const ds = await diagnostics(uri, 60_000);
  const semi = ds.find((d) => d.code === 'UnnecessarySemicolon');
  assert.ok(semi, `no UnnecessarySemicolon in ${JSON.stringify(ds.map((d) => d.code))}`);
  assert.equal(semi.range.start.line, 2);

  await vscode.commands.executeCommand('editor.action.formatDocument');
  assert.equal(doc.getText(), 'class A {\n    def f() {\n        println 1;\n    }\n}\n');

  const actions = await vscode.commands.executeCommand<vscode.CodeAction[]>(
    'vscode.executeCodeActionProvider',
    uri,
    new vscode.Range(0, 0, doc.lineCount, 0),
    'source.fixAll.groovyfmt',
  );
  const fixAll = actions.find((a) => a.kind?.value === 'source.fixAll.groovyfmt');
  assert.ok(fixAll?.edit, `no fix-all action in ${JSON.stringify(actions.map((a) => a.title))}`);
  assert.ok(await vscode.workspace.applyEdit(fixAll.edit));
  assert.equal(doc.getText(), 'class A {\n    def f() {\n        println 1\n    }\n}\n');
  console.log('integration: diagnostics, format document and fix all passed');
}
