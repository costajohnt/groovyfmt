// Runs inside VS Code (see run.ts): a lint diagnostic arrives, format document fixes the
// indentation, and the fix-all code action removes the semicolon.
import assert from 'node:assert/strict';
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

export async function run(): Promise<void> {
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
