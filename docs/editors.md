# Editors

Editors use groovyfmt in one of two ways. A language server client runs `groovyfmt --lsp`. Any other editor pipes the buffer through `groovyfmt --stdin`.

## Language server

`groovyfmt --lsp` is a language server on stdin and stdout. An editor that speaks the Language Server Protocol gets these from the same binary the CLI uses:

- formatting, of the whole file and of a selection
- parse errors and lint violations as you type
- the `--lint --fix` fixes as code actions: "Fix all auto-fixable problems" (kind `source.fixAll.groovyfmt`) and a quick fix per fixable violation

How it treats documents:

- Each document gets the config `--stdin-filepath` would find for its path, read again on every request, so an edited `.groovyfmt.toml` applies right away.
- An excluded file gets no diagnostics and no edits.
- A buffer that doesn't parse gets its parse errors and no edits.
- `--config` and `--indent` apply as on the command line.
- Only `file:` documents are served, so an unsaved buffer with no path gets nothing.
- It never writes a file. The edits go back to the client, which applies them.
- It honors the config's lint [baseline](lint.md#baseline).

It is a formatter and linter server. There is no completion, hover, go-to-definition or rename.

### Neovim

Neovim 0.11 or later, with the built-in client:

```lua
vim.lsp.config("groovyfmt", {
  cmd = { "groovyfmt", "--lsp" },
  filetypes = { "groovy" },
  root_markers = { ".groovyfmt.toml", "groovyfmt.toml", ".git" },
})
vim.lsp.enable("groovyfmt")
```

`vim.lsp.buf.format()` formats. `vim.lsp.buf.code_action({ context = { only = { "source.fixAll" } }, apply = true })` applies the fixes.

### VS Code

The groovyfmt extension (`costajohnt.groovyfmt`, source in [`editors/vscode`](../editors/vscode)) starts the language server with no setup. It covers `*.groovy`, `*.gvy`, `*.gy`, `*.gsh`, `*.gradle`, `*.jenkinsfile`, `Jenkinsfile` and `Jenkinsfile.*`, alongside VS Code's built-in Groovy support or any other Groovy, Gradle or Jenkins extension.

It runs `groovyfmt.path` if set, else the workspace's npm package (`node_modules`), else `groovyfmt` on `PATH`. With none of those, it offers to download the release matching its own version, checked against the SHA-256 checksums that release of the extension carries (taken from the archives when the release was built). In an untrusted workspace it ignores `groovyfmt.path` from workspace settings and `node_modules`, and a relative `groovyfmt.path` in user settings is refused. Its [README](../editors/vscode/README.md) has the details.

Until it is on the Marketplace and Open VSX, install the `.vsix` attached to a [release](https://github.com/costajohnt/groovyfmt/releases) with **Extensions: Install from VSIX...** or `code --install-extension groovyfmt-<version>.vsix`.

Format on save is `editor.formatOnSave`. To apply the lint fixes on save too:

```json
"[groovy]": {
  "editor.defaultFormatter": "costajohnt.groovyfmt",
  "editor.formatOnSave": true,
  "editor.codeActionsOnSave": { "source.fixAll.groovyfmt": "explicit" }
}
```

### IntelliJ IDEA, Android Studio and other JetBrains IDEs

The groovyfmt plugin (`io.github.costajohnt.groovyfmt`, source in [`editors/jetbrains`](../editors/jetbrains)) starts the language server through [LSP4IJ](https://plugins.jetbrains.com/plugin/23257-lsp4ij), Red Hat's open source language server client, which it depends on. It runs in any IDE based on IntelliJ Platform 2024.2 or later, Community Edition and Android Studio included, and covers the same files as the VS Code extension, by file name, alongside the bundled Groovy and Gradle support and any Jenkins plugin. Diagnostics and quick fixes (each fixable violation, and "Fix all auto-fixable problems") appear as you type. **Code | Reformat with groovyfmt** formats the file.

By default **Reformat Code** keeps the IDE's own Groovy formatter, since replacing it without asking would change what a familiar shortcut does. The setting **Use groovyfmt for Reformat Code** (Settings | Tools | groovyfmt) hands Reformat Code to groovyfmt. Format on save is then the IDE's own **Actions on Save | Reformat code**.

It looks for groovyfmt in the same order as the VS Code extension. A path set for the project in `.idea/groovyfmt.xml` comes first, then the IDE-wide path, which must be absolute, then the project's `node_modules`, then `PATH`. The project's path and `node_modules` count only in a trusted project, so in a project the IDE opened in safe mode nothing from the project decides which binary runs. With none of those, a release build of the plugin offers to download the release matching its own version, checked against the SHA-256 checksums packaged into the plugin. Its [README](../editors/jetbrains/README.md) has the details.

Until it is on JetBrains Marketplace, install LSP4IJ from the Marketplace, then the `groovyfmt-jetbrains-<version>.zip` attached to a [release](https://github.com/costajohnt/groovyfmt/releases) with **Settings | Plugins | ⚙ | Install Plugin from Disk...**.

#### LSP4IJ alone

With LSP4IJ installed, groovyfmt also runs as an LSP4IJ user-defined language server, with no groovyfmt plugin. In **Settings | Languages & Frameworks | Language Servers**, add a server with **+**:

- **Server** tab: name `groovyfmt`, command `/path/to/groovyfmt --lsp` (the full path: an IDE started from the Dock or a launcher may not see your shell's `PATH`).
- **Mappings** tab, **File name patterns**: the patterns `*.groovy;*.gvy;*.gy;*.gsh;*.gradle;*.jenkinsfile;Jenkinsfile;Jenkinsfile.*` with the **Language ID** `groovy`.
- **Configuration** tab, **Client**: `{"format": {"existingFormatterOverrideable": true}}` if Reformat Code should run groovyfmt in place of the IDE's Groovy formatter. Without it, Reformat Code runs groovyfmt only in files the IDE has no formatter for.

Diagnostics, quick fixes and Reformat Code then come from groovyfmt. The plugin's integration tests run this setup, as LSP4IJ 0.21.0 creates it, in IntelliJ IDEA Community 2024.2. It runs whatever binary the command names, in every project, trusted or not, and nothing downloads groovyfmt for you.

#### Without a language server

In an IDE older than 2024.2, run groovyfmt on the file itself.

- To format on save, open Settings, Tools, File Watchers, and add a watcher for Groovy files with program `groovyfmt` and arguments `$FilePath$`.
- To format on demand, add an External Tool (Settings, Tools, External Tools) with program `groovyfmt`, arguments `$FilePath$` and working directory `$ProjectFileDir$`, then bind it to a key.

Either way the file is named explicitly, so `exclude` patterns are honored.

### Helix

Helix knows Groovy, `*.gradle` and Jenkinsfiles, and has no Groovy language server by default. In `languages.toml`:

```toml
[language-server.groovyfmt]
command = "groovyfmt"
args = ["--lsp"]

[[language]]
name = "groovy"
language-servers = ["groovyfmt"]
auto-format = true
```

### Zed

Zed can't add a language server from settings alone. It starts the ones its extensions declare. The [Groovy extension](https://github.com/valentinegb/zed-groovy) declares a server named `groovy`; point that server at groovyfmt in `settings.json` (this replaces the extension's own server):

```json
"lsp": {
  "groovy": { "binary": { "path": "/usr/local/bin/groovyfmt", "arguments": ["--lsp"] } }
},
"languages": {
  "Groovy": { "formatter": "language_server", "format_on_save": "on" }
}
```

## Formatting through stdin

Without a language server client, call `groovyfmt --stdin`. It prints nothing and exits 2 when it can't format the buffer, so a parse error never replaces the buffer's code.

Add `--stdin-filepath <path>` (it requires `--stdin`) to tell groovyfmt which file the buffer belongs to.

- The config is then found by walking up from that file's directory, as for a file named on the command line, instead of from the working directory.
- The path names the buffer in error messages.
- If the path matches `exclude`, the buffer comes back unchanged with exit 0.
- The file doesn't have to exist yet.

### Neovim with conform.nvim

[conform.nvim](https://github.com/stevearc/conform.nvim):

```lua
require("conform").setup({
  formatters = {
    groovyfmt = { command = "groovyfmt", args = { "--stdin", "--stdin-filepath", "$FILENAME" } },
  },
  formatters_by_ft = { groovy = { "groovyfmt" } },
})
```

### VS Code with Custom Local Formatters

[Custom Local Formatters](https://marketplace.visualstudio.com/items?itemName=jkillian.custom-local-formatters):

```json
"customLocalFormatters.formatters": [
  { "command": "groovyfmt --stdin", "languages": ["groovy"] }
]
```
