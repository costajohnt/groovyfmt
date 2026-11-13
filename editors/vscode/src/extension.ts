import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import * as vscode from 'vscode';
import { LanguageClient, State, type DocumentSelector } from 'vscode-languageclient/node';
import {
  MIN_LSP_VERSION,
  TARGETS,
  download,
  downloadPath,
  embeddedSha256,
  findBinary,
  isOlder,
  parseVersion,
  runVersion,
  settingValues,
  type Checksums,
  type Found,
} from './binary';

// The files groovyfmt formats, matched by path whatever language id another extension gives
// them (groovy, gradle, jenkins, jenkinsfile). Only file: documents: the server ignores others.
const SELECTOR: DocumentSelector = [
  '**/*.groovy',
  '**/*.gvy',
  '**/*.gy',
  '**/*.gsh',
  '**/*.gradle',
  '**/*.jenkinsfile',
  '**/Jenkinsfile',
  '**/Jenkinsfile.*',
].map((pattern) => ({ scheme: 'file', pattern }));

const DECLINED = 'groovyfmt.downloadDeclined';

let ctx: vscode.ExtensionContext;
let output: vscode.LogOutputChannel;
let status: vscode.StatusBarItem;
let client: LanguageClient | undefined;
// starts and stops run one at a time, in order
let queue: Promise<void> = Promise.resolve();
// the download offer runs beside the queue, one at a time
let offering = false;

function problem(message: string): void {
  output.error(message);
  status.tooltip = `${message}\nClick to show the groovyfmt output.`;
  status.show();
}

function restart(): Promise<void> {
  queue = queue.then(stop).then(start).catch((e: unknown) => problem(`groovyfmt: ${String(e)}`));
  return queue;
}

export function activate(context: vscode.ExtensionContext): void {
  ctx = context;
  output = vscode.window.createOutputChannel('groovyfmt', { log: true });
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right);
  status.text = '$(error) groovyfmt';
  status.command = 'groovyfmt.showOutput';
  context.subscriptions.push(
    output,
    status,
    vscode.commands.registerCommand('groovyfmt.restartServer', async () => {
      // an explicit restart may ask about the download again
      await context.globalState.update(DECLINED, undefined);
      await restart();
    }),
    vscode.commands.registerCommand('groovyfmt.showOutput', () => output.show()),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('groovyfmt.enable') || e.affectsConfiguration('groovyfmt.path')) void restart();
    }),
    vscode.workspace.onDidGrantWorkspaceTrust(() => void restart()),
    vscode.workspace.onDidChangeWorkspaceFolders(() => void restart()),
  );
  // not awaited: activation never waits on the server, the prompt or a download
  void restart();
}

export async function deactivate(): Promise<void> {
  await stop();
}

async function stop(): Promise<void> {
  const c = client;
  client = undefined;
  status.hide();
  await c?.stop(2000).catch(() => undefined);
}

async function start(): Promise<void> {
  if (!vscode.workspace.getConfiguration('groovyfmt').get<boolean>('enable', true)) {
    output.info('groovyfmt.enable is off');
    return;
  }
  const folders = (vscode.workspace.workspaceFolders ?? []).filter((f) => f.uri.scheme === 'file');
  // inspected for the first folder, so its own folder settings count
  const path = vscode.workspace.getConfiguration('groovyfmt', folders[0]?.uri).inspect<string>('path');
  const found =
    findBinary({
      setting: settingValues(path),
      trusted: vscode.workspace.isTrusted,
      folders: folders.map((f) => f.uri.fsPath),
      envPath: process.env.PATH ?? '',
      platform: process.platform,
      arch: process.arch,
    }) ?? downloaded();
  if (!found) {
    void offerDownload();
    return;
  }
  if ('problem' in found) return problem(found.problem);

  let version: number[] | undefined;
  try {
    version = parseVersion(await runVersion(found.path));
  } catch (e) {
    return problem(`Could not run groovyfmt at ${found.path} (from ${found.source}): ${String(e)}`);
  }
  if (!version) return problem(`${found.path} (from ${found.source}) did not print a groovyfmt version.`);
  const v = version.join('.');
  if (isOlder(version, MIN_LSP_VERSION.split('.').map(Number))) {
    return problem(
      `groovyfmt ${v} at ${found.path} (from ${found.source}) is too old: the language server ` +
        `(--lsp) arrived in ${MIN_LSP_VERSION}. Install a newer groovyfmt.`,
    );
  }
  output.info(`Starting groovyfmt ${v} from ${found.source}: ${found.path}`);

  const c = new LanguageClient(
    'groovyfmt',
    'groovyfmt',
    { command: found.path, args: ['--lsp'], options: { cwd: folders[0]?.uri.fsPath } },
    { documentSelector: SELECTOR, outputChannel: output },
  );
  client = c;
  c.onDidChangeState((e) => {
    // a stop we asked for has already replaced `client`
    if (client !== c) return;
    if (e.newState === State.Running) status.hide();
    else if (e.newState === State.Stopped) problem('The groovyfmt language server stopped. Run "groovyfmt: Restart server" to start it again.');
  });
  try {
    await c.start();
  } catch (e) {
    problem(`The groovyfmt language server failed to start: ${String(e)}`);
  }
}

const version = (): string => ctx.extension.packageJSON.version;
const dest = (): string => downloadPath(ctx.globalStorageUri.fsPath, version(), process.platform, process.arch);

/** The binary this extension downloaded earlier for its version. */
function downloaded(): Found | undefined {
  return existsSync(dest()) ? { path: dest(), source: 'download' } : undefined;
}

/** The checksums a release build carries in dist/checksums.json; none in other builds. */
function checksums(): Checksums | undefined {
  try {
    return JSON.parse(readFileSync(join(ctx.extensionPath, 'dist', 'checksums.json'), 'utf8'));
  } catch {
    return undefined;
  }
}

/** Offers the download, and restarts the server once it is in place. */
async function offerDownload(): Promise<void> {
  if (offering) return;
  offering = true;
  try {
    const v = version();
    const { platform, arch } = process;
    const missing = 'groovyfmt was not found in groovyfmt.path, node_modules or PATH.';
    if (!TARGETS[`${platform}-${arch}`]) {
      return problem(`${missing} There is no prebuilt binary for ${platform}-${arch}; install groovyfmt and set groovyfmt.path.`);
    }
    const sha256 = embeddedSha256(checksums(), v, platform, arch);
    if (!sha256) {
      return problem(
        `${missing} This build of the extension carries no checksums for groovyfmt ${v} (only release builds do), ` +
          'so it does not download groovyfmt. Install groovyfmt another way and set groovyfmt.path if it is not on PATH, ' +
          'or install a release build of the extension.',
      );
    }
    if (ctx.globalState.get(DECLINED)) {
      return problem(`${missing} Install it, set groovyfmt.path, or run "groovyfmt: Restart server" to be offered the download again.`);
    }
    const choice = await vscode.window.showInformationMessage(
      `${missing} Download groovyfmt ${v} for ${platform}-${arch} from its GitHub release? ` +
        'Before it runs, the download is checked against the SHA-256 checksum that this extension release carries.',
      'Download',
      'Not now',
    );
    if (choice === 'Not now') await ctx.globalState.update(DECLINED, true);
    if (choice !== 'Download') return problem(`${missing} The download was declined.`);
    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: `Downloading groovyfmt ${v}` },
      () => download(v, platform, arch, dest(), undefined, { sha256 }),
    );
    output.info(`Downloaded groovyfmt ${v} to ${dest()}, SHA-256 matches the checksum this extension carries`);
    void restart();
  } catch (e) {
    problem(`Downloading groovyfmt failed: ${String(e)}`);
  } finally {
    offering = false;
  }
}
