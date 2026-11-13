# groovyfmt for JetBrains IDEs

Formats and lints Groovy, Gradle build scripts and Jenkinsfiles with [groovyfmt](https://github.com/costajohnt/groovyfmt), a fast formatter and linter written in Rust, in IntelliJ IDEA, Android Studio and the other JetBrains IDEs. The plugin starts `groovyfmt --lsp`, the language server built into the groovyfmt binary, through [LSP4IJ](https://plugins.jetbrains.com/plugin/23257-lsp4ij), Red Hat's open source language server client, so the IDE gives the same results as the command line.

- parse errors and lint violations as you type, with a quick fix for each fixable violation and "Fix all auto-fixable problems"
- **Code | Reformat with groovyfmt** formats the file with groovyfmt
- optionally, **Reformat Code** uses groovyfmt, and with it the IDE's **Reformat code** action on save

It covers `*.groovy`, `*.gvy`, `*.gy`, `*.gsh`, `*.gradle`, `*.jenkinsfile`, `Jenkinsfile` and `Jenkinsfile.*`, by file name, whatever file type another plugin gives them. It has no highlighting, completion or navigation of its own: the bundled Groovy and Gradle support and any Jenkins plugin keep providing those, and the plugin works alongside them.

It needs an IDE based on IntelliJ Platform 2024.2 or later (LSP4IJ's minimum), including IntelliJ IDEA Community Edition and Android Studio.

## Install

Until the plugin is on JetBrains Marketplace, install LSP4IJ from the Marketplace (**Settings | Plugins**), then the `groovyfmt-jetbrains-<version>.zip` attached to a [release](https://github.com/costajohnt/groovyfmt/releases) with **Settings | Plugins | ⚙ | Install Plugin from Disk...**.

Then open a Groovy file. If groovyfmt isn't installed, the plugin offers to download the release that matches its own version from GitHub and keeps it in the IDE's system directory. Each release of the plugin carries the SHA-256 of that release's archives, taken when they were built, and a download that doesn't match is never written or run. A build of the plugin made outside a release (from a pull request, or built by hand) carries no checksums and doesn't download; install groovyfmt yourself to use it.

## Reformat Code and the IDE's Groovy formatter

The plugin adds a formatter and takes nothing away, so by default **Reformat Code** in a Groovy or Gradle file still runs the IDE's own Groovy formatter, and **Code | Reformat with groovyfmt** runs groovyfmt. Turn on **Use groovyfmt for Reformat Code** in the settings to have Reformat Code run groovyfmt instead. A file the IDE has no formatter for, such as a Jenkinsfile without a Jenkins plugin, is formatted by groovyfmt either way.

To format on save, turn on **Use groovyfmt for Reformat Code** and the IDE's **Settings | Tools | Actions on Save | Reformat code**.

## Settings

**Settings | Tools | groovyfmt**:

| Setting | Default | |
|---|---|---|
| Run the groovyfmt language server | on | |
| Path to groovyfmt | empty | For every project. It must be an absolute path. |
| Path for this project | empty | Stored in `.idea/groovyfmt.xml`, absolute or relative to the project directory. Used over the path above, and only in a trusted project. |
| Use groovyfmt for Reformat Code | off | See above. |

Everything about formatting and linting (indentation, line length, which rules run, excluded files, the lint baseline) lives in groovyfmt's config file, `.groovyfmt.toml`, the same one the command line reads. The server finds the config for each file by walking up from its directory and reads it again on every request, so an edit to `.groovyfmt.toml` applies right away. See [Configuration](https://github.com/costajohnt/groovyfmt/blob/main/docs/configuration.md) and [Lint](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md).

## Finding groovyfmt

The plugin runs the first of these that exists:

1. The path for this project, in a trusted project.
2. The path to groovyfmt from the IDE-wide settings.
3. The project's npm package, in a trusted project: `node_modules/@groovyfmt/<platform>/bin/groovyfmt`, then `node_modules/.bin/groovyfmt`.
4. `groovyfmt` on `PATH` (absolute entries only).
5. The release binary it downloaded before, for this plugin's version.
6. Otherwise, in a release build, it asks whether to download that binary. "Not now" is remembered; **Tools | Restart groovyfmt** asks again. Dismissing the notification without choosing asks again next time.

The language server arrived in groovyfmt 0.9.0. An older binary is reported instead of started.

### Trusted projects

In a project the IDE opened in safe mode (not trusted), the plugin never runs a binary the project chose: it ignores the path in `.idea/groovyfmt.xml` and the project's `node_modules`. The IDE-wide path, `PATH` and the downloaded binary still apply. The IDE-wide path must be absolute and relative `PATH` entries are skipped, in every project, since either would resolve inside whatever project is open. When you trust the project, the plugin finds the binary again with the project's choice.

## Commands

- **Code | Reformat with groovyfmt** formats the file in the editor with groovyfmt (`groovyfmt --stdin --stdin-filepath <file>`), with the binary the language server runs.
- **Tools | Restart groovyfmt** finds the binary again and restarts the language server.

When something is wrong (no binary, a binary that is too old), a groovyfmt notification says what and offers the settings. LSP4IJ's LSP console tool window shows the server's state and its log.

## Privacy

No telemetry. The only network request is the release download, made after you agree to it.

## Development

`./gradlew test` runs the unit tests and the integration tests in IntelliJ IDEA Community 2024.2, which need a groovyfmt build named by `GROOVYFMT_BIN` (an absolute path). `./gradlew verifyPlugin` runs the JetBrains Plugin Verifier against the IDEs listed in `build.gradle.kts`. Both need JDK 21.

## License

MIT
