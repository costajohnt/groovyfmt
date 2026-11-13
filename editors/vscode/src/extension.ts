import { existsSync } from 'node:fs';
import * as vscode from 'vscode';
import { LanguageClient, State, type DocumentSelector } from 'vscode-languageclient/node';
import {
  MIN_LSP_VERSION,
  TARGETS,
  download,
  downloadPath,
  findBinary,
  isOlder,
  parseVersion,
  runVersion,
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

function problem(message: string): void {
  output.error(message);
  status.tooltip = `${message}\nClick to show the groovyfmt output.`;
  status.show();
}

function restart(): Promise<void> {
  queue = queue.then(stop).then(start).catch((e: unknown) => problem(`groovyfmt: ${String(e)}`));
  return queue;
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
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
  await restart();
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
  const config = vscode.workspace.getConfiguration('groovyfmt');
  if (!config.get<boolean>('enable', true)) {
    output.info('groovyfmt.enable is off');
    return;
  }
  const path = config.inspect<string>('path');
  const folders = (vscode.workspace.workspaceFolders ?? [])
    .filter((f) => f.uri.scheme === 'file')
    .map((f) => f.uri.fsPath);
  const found =
    findBinary({
      setting: { user: path?.globalValue, workspace: path?.workspaceFolderValue || path?.workspaceValue },
      trusted: vscode.workspace.isTrusted,
      folders,
      envPath: process.env.PATH ?? '',
      platform: process.platform,
      arch: process.arch,
    }) ?? (await downloaded());
  if (!found) return;

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
    { command: found.path, args: ['--lsp'], options: { cwd: folders[0] } },
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

/** The binary downloaded for this extension's version, downloading it if the user agrees. */
async function downloaded(): Promise<Found | undefined> {
  const version: string = ctx.extension.packageJSON.version;
  const { platform, arch } = process;
  const dest = downloadPath(ctx.globalStorageUri.fsPath, version, platform, arch);
  if (existsSync(dest)) return { path: dest, source: 'download' };
  const missing = 'groovyfmt was not found in groovyfmt.path, node_modules or PATH.';
  if (!TARGETS[`${platform}-${arch}`]) {
    problem(`${missing} There is no prebuilt binary for ${platform}-${arch}; install groovyfmt and set groovyfmt.path.`);
    return undefined;
  }
  if (ctx.globalState.get(DECLINED)) {
    problem(`${missing} Install it, set groovyfmt.path, or run "groovyfmt: Restart server" to be offered the download again.`);
    return undefined;
  }
  const choice = await vscode.window.showInformationMessage(
    `${missing} Download groovyfmt ${version} for ${platform}-${arch} from its GitHub release? ` +
      'The download is checked against the release checksum before it runs.',
    'Download',
    'Not now',
  );
  if (choice !== 'Download') {
    await ctx.globalState.update(DECLINED, true);
    problem(`${missing} The download was declined.`);
    return undefined;
  }
  try {
    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: `Downloading groovyfmt ${version}` },
      () => download(version, platform, arch, dest),
    );
  } catch (e) {
    problem(`Downloading groovyfmt ${version} failed: ${String(e)}`);
    return undefined;
  }
  output.info(`Downloaded groovyfmt ${version} to ${dest}, checksum verified`);
  return { path: dest, source: 'download' };
}
