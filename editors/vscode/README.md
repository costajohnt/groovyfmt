# groovyfmt for VS Code

Formats and lints Groovy, Gradle build scripts and Jenkinsfiles with [groovyfmt](https://github.com/costajohnt/groovyfmt), a fast formatter and linter written in Rust. The extension starts `groovyfmt --lsp`, the language server built into the groovyfmt binary, so the editor gives the same results as the command line.

- Format Document and Format Selection, and format on save through VS Code's own settings
- parse errors and lint violations as you type
- a quick fix for each fixable violation, and "Fix all auto-fixable problems" (`source.fixAll.groovyfmt`)

It covers `*.groovy`, `*.gvy`, `*.gy`, `*.gsh`, `*.gradle`, `*.jenkinsfile`, `Jenkinsfile` and `Jenkinsfile.*`. It has no syntax highlighting, completion or navigation of its own. VS Code's built-in Groovy support or another Groovy, Gradle or Jenkins extension provides those, and this extension works alongside them, whatever language id they give these files.

## Install

Install the extension, then open a Groovy file. If groovyfmt isn't installed, the extension offers once to download the release that matches its own version from GitHub, checks it against the release's SHA-256 checksum, and keeps it in the extension's storage.

To use a groovyfmt you installed yourself, see [Finding groovyfmt](#finding-groovyfmt). The groovyfmt [README](https://github.com/costajohnt/groovyfmt#install) lists the ways to install it (Homebrew, npm, release archives, cargo).

## Formatting on save

Format on save is VS Code's `editor.formatOnSave`. If another extension also formats Groovy, make groovyfmt the default formatter. To apply the lint fixes on save too, add `source.fixAll.groovyfmt` to `editor.codeActionsOnSave`:

```json
"[groovy]": {
  "editor.defaultFormatter": "costajohnt.groovyfmt",
  "editor.formatOnSave": true,
  "editor.codeActionsOnSave": { "source.fixAll.groovyfmt": "explicit" }
}
```

A Gradle or Jenkins extension may give `*.gradle` files or Jenkinsfiles a language id of its own (`gradle`, `jenkins` or `jenkinsfile`); add the same block for that id to format them on save.

## Settings

| Setting | Default | |
|---|---|---|
| `groovyfmt.enable` | `true` | Run the language server. |
| `groovyfmt.path` | `""` | Path to the `groovyfmt` binary. A relative path is resolved against the first workspace folder. |

Everything about formatting and linting (indentation, line length, which rules run, excluded files, the lint baseline) lives in groovyfmt's config file, `.groovyfmt.toml`, the same one the command line reads. The server finds the config for each file by walking up from its directory and reads it again on every request, so an edit to `.groovyfmt.toml` applies right away. To turn lint rules off, turn them off in the config; see [Configuration](https://github.com/costajohnt/groovyfmt/blob/main/docs/configuration.md) and [Lint](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md).

## Finding groovyfmt

The extension runs the first of these that exists:

1. `groovyfmt.path`.
2. The workspace's npm package: `node_modules/@groovyfmt/<platform>/bin/groovyfmt`, then `node_modules/.bin/groovyfmt`, in each workspace folder.
3. `groovyfmt` on `PATH`.
4. The release binary it downloaded before, for this extension's version.
5. Otherwise it asks whether to download that binary. Declining is remembered; run **groovyfmt: Restart server** to be asked again.

The language server arrived in groovyfmt 0.9.0. An older binary is reported instead of started.

### Workspace Trust

In a [restricted (untrusted) workspace](https://code.visualstudio.com/docs/editor/workspace-trust), the extension never runs a binary that the workspace chose: it ignores `groovyfmt.path` from the workspace's settings and the workspace's `node_modules`. `groovyfmt.path` from your user settings, `PATH` and the downloaded binary still apply. When you trust the workspace, the server restarts with the workspace's choice.

## Commands

- **groovyfmt: Restart server** stops the server, finds the binary again and starts it.
- **groovyfmt: Show output** opens the extension's log, which names the binary in use and where it came from.

When something is wrong (no binary, a binary that is too old, a server that stopped), a groovyfmt item appears in the status bar. Click it for the log.

## Privacy

No telemetry. The only network request is the release download, made after you agree to it.

## License

MIT
