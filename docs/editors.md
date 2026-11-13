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

There is no groovyfmt extension, and VS Code itself only runs language servers that an extension starts. A generic client such as [Generic LSP Client](https://github.com/llllvvuu/vscode-glspc) (`llllvvuu.glspc`) can start it. In `settings.json`:

```json
"glspc.serverCommand": "groovyfmt",
"glspc.serverCommandArguments": ["--lsp"],
"glspc.languageId": "groovy"
```

To fix on save, add `"editor.codeActionsOnSave": { "source.fixAll.groovyfmt": "explicit" }`.

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

### IntelliJ IDEA

IntelliJ runs groovyfmt on the file itself.

- To format on save, open Settings, Tools, File Watchers, and add a watcher for Groovy files with program `groovyfmt` and arguments `$FilePath$`.
- To format on demand, add an External Tool (Settings, Tools, External Tools) with program `groovyfmt`, arguments `$FilePath$` and working directory `$ProjectFileDir$`, then bind it to a key.

Either way the file is named explicitly, so `exclude` patterns are honored.
