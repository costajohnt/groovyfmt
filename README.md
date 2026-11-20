<img src="https://raw.githubusercontent.com/costajohnt/groovyfmt/main/docs/banner.png" width="100%" alt="groovyfmt: fast formatter and linter for Groovy, Gradle and Jenkinsfiles">

A fast formatter and linter for Groovy, Gradle build scripts and Jenkinsfiles. One static binary, no JVM. Written in Rust on a [tree-sitter](https://tree-sitter.github.io/) grammar.

By default the formatter re-indents every line from the parse tree, trims trailing whitespace and collapses runs of blank lines; spacing within lines and line wrapping are opt-in ([formatting](https://github.com/costajohnt/groovyfmt/blob/main/docs/formatting.md)).

```console
$ groovyfmt --check --diff build.gradle
--- a/build.gradle
+++ b/build.gradle
@@ -1,8 +1,8 @@
 dependencies {
-        implementation 'org.slf4j:slf4j-api:2.0.9'
+    implementation 'org.slf4j:slf4j-api:2.0.9'
 }
 tasks.register('hello') {
-  doLast {
-println 'hello'
-      }
+    doLast {
+        println 'hello'
+    }
 }
would reformat 1 of 1 files
```

```console
$ groovyfmt --check
Jenkinsfile
test/groovy/BaseTest.groovy
...
would reformat 27 of 27 files
$ groovyfmt
reformatted 27 of 27 files
$ groovyfmt --lint
test/groovy/BaseTest.groovy:2:1: UnusedImport: The [mock.CurrentBuild] import is never referenced (fixable)
...
10 violations in 27 files (UnnecessaryGroovyImport 1, UnusedImport 9)
```

<sub>Run on [jenkins-infra/pipeline-library](https://github.com/jenkins-infra/pipeline-library).</sub>

- **Safe.** Formatting changes whitespace only by default. Output is re-parsed and token-compared with the input, and a file is left untouched if anything else would change. Files that don't parse are never touched.
- **Fast.** Formats the 635-file [spock](https://github.com/spockframework/spock) repo in 0.19 s (measured on 0.5). npm-groovy-lint didn't finish in 15 minutes ([benchmark](https://github.com/costajohnt/groovyfmt/blob/main/docs/coverage.md#speed)).
- **Compared.** [Spotless](https://github.com/diffplug/spotless) formats Groovy with the Eclipse formatter (greclipse), on the JVM inside a Gradle or Maven build. [npm-groovy-lint](https://github.com/nvuillam/npm-groovy-lint) runs CodeNarc on a JVM. groovyfmt runs without a JVM or a build.
- **Broad.** 99.6% of a 23,766-file corpus (grails-core, gradle, spock, nextflow, Jenkins pipelines) parses ([coverage](https://github.com/costajohnt/groovyfmt/blob/main/docs/coverage.md)).

## Install

```sh
brew install costajohnt/tap/groovyfmt       # macOS, Linux
npm install --save-dev groovyfmt            # or: npx groovyfmt --check
cargo install --locked groovyfmt            # Rust 1.90+ and a C compiler
docker run --rm -v "$PWD:/src" -w /src ghcr.io/costajohnt/groovyfmt --check
```

Prebuilt binaries for Linux (static musl), macOS and Windows, each with a `.sha256`, are on every [release](https://github.com/costajohnt/groovyfmt/releases). See also [Windows](https://github.com/costajohnt/groovyfmt/blob/main/docs/windows.md).

In a Gradle build, the [Gradle plugin](https://github.com/costajohnt/groovyfmt/tree/main/gradle-plugin) adds `groovyfmtCheck` and `groovyfmtLint` to `check`: `id 'io.github.costajohnt.groovyfmt' version '0.30.5'`.

## Usage

| Command | Does |
|---|---|
| `groovyfmt` | format files under the current directory in place |
| `groovyfmt --check` | list files that would change, exit 1 if any (`--diff` to show them) |
| `groovyfmt --lint` | report lint violations, exit 1 if any |
| `groovyfmt --lint --fix` | fix what the rules mark [fixable](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md#fixing) (imports, semicolons, removed Gradle syntax and more), report the rest |
| `groovyfmt --syntax` | report parse errors |
| `groovyfmt --explain <Rule>` | explain a lint rule |
| `groovyfmt --suggest-style` | suggest indent settings that fit existing code |

Exit status: 0 clean, 1 something to fix, 2 error. Picks up `*.groovy`, `*.gradle`, `*.gvy`, `*.gy`, `*.gsh`, Jenkinsfiles and groovy `#!` scripts. Kotlin DSL scripts (`build.gradle.kts`) are not handled. [Command line](https://github.com/costajohnt/groovyfmt/blob/main/docs/usage.md) has every option.

## Configuration

`.groovyfmt.toml`, found by walking up from each file:

```toml
indent = 4
continuation_indent = 8     # default: same as indent
use_tabs = false
normalize_spacing = false   # also fix spaces between tokens
max_line_length = 120       # wrap long lines where safe (default: off)
exclude = ["generated/**"]

[lint]                      # turn rules on or off
UnnecessaryGString = true
```

`// groovyfmt: off` / `// groovyfmt: on` protect a region. More in [Formatting](https://github.com/costajohnt/groovyfmt/blob/main/docs/formatting.md) and [Configuration](https://github.com/costajohnt/groovyfmt/blob/main/docs/configuration.md).

## Linting

89 rules, 65 on by default (`groovyfmt --rules`).
- General rules follow [CodeNarc](https://codenarc.org/) names. `groovyfmt --import-codenarc <config>` converts a CodeNarc or npm-groovy-lint config.
- Jenkinsfile, Gradle and Spock rules catch mistakes that otherwise surface only when a build runs, such as Gradle 9 and Groovy 4 removals.
- `// groovyfmt-ignore` or `@SuppressWarnings` suppress a report; `--write-baseline` lets you adopt lint gradually.

```console
$ groovyfmt --lint
build.gradle:3:1: GradleRemovedApi: The project property `sourceCompatibility` was removed in Gradle 9.0 with the plugin conventions; use java { sourceCompatibility = ... }
```

[Lint](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md) has every rule. [Output formats](https://github.com/costajohnt/groovyfmt/blob/main/docs/output-formats.md) covers JSON, SARIF and GitHub annotations.

## CI

```yaml
- uses: actions/checkout@v5
- uses: costajohnt/groovyfmt@v0.30.5      # runs groovyfmt --check --output-format github
- run: groovyfmt --lint --output-format github
```

pre-commit:

```yaml
repos:
  - repo: https://github.com/costajohnt/groovyfmt
    rev: v0.30.5
    hooks:
      - id: groovyfmt        # or groovyfmt-check
      - id: groovyfmt-lint
```

[CI](https://github.com/costajohnt/groovyfmt/blob/main/docs/ci.md) covers action inputs, SARIF upload, `--strict` and the Gradle plugin.

## Editors and agents

`groovyfmt --lsp` is a language server: formatting, parse errors and lint as you type, fixes as code actions. Extensions for [VS Code](https://github.com/costajohnt/groovyfmt/tree/main/editors/vscode) and [JetBrains IDEs](https://github.com/costajohnt/groovyfmt/tree/main/editors/jetbrains); setup for Neovim, Helix and Zed in [Editors](https://github.com/costajohnt/groovyfmt/blob/main/docs/editors.md).

Coding agents editing a Jenkinsfile have no compiler to catch a broken file. `groovyfmt --syntax <file>` answers in milliseconds. [Coding agents](https://github.com/costajohnt/groovyfmt/blob/main/docs/agents.md) has an `AGENTS.md` snippet and a Claude Code hook.

## Contributing

See [Development](https://github.com/costajohnt/groovyfmt/blob/main/docs/development.md) for tests, the corpus workflow, grammar changes and releases.

## License

MIT
