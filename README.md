<img src="https://raw.githubusercontent.com/costajohnt/groovyfmt/main/docs/banner.png" width="100%" alt="groovyfmt: fast formatter and linter for Groovy, Gradle and Jenkinsfiles">

A fast formatter and linter for Groovy, Gradle build scripts and Jenkinsfiles. One static binary, no JVM. Written in Rust on a [tree-sitter](https://tree-sitter.github.io/) grammar.

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
- **Fast.** Formats the 635-file [spock](https://github.com/spockframework/spock) repo in 0.19 s. npm-groovy-lint didn't finish in 15 minutes ([benchmark](https://github.com/costajohnt/groovyfmt/blob/main/docs/coverage.md#speed)).
- **Broad.** 99.6% of a 23,766-file corpus (grails-core, gradle, spock, nextflow, Jenkins pipelines) parses ([coverage](https://github.com/costajohnt/groovyfmt/blob/main/docs/coverage.md)).

## Install

```sh
brew install costajohnt/tap/groovyfmt       # macOS, Linux
npm install --save-dev groovyfmt            # or: npx groovyfmt --check
cargo install --locked groovyfmt            # Rust 1.90+ and a C compiler
```

Prebuilt binaries for Linux (static musl), macOS and Windows, each with a `.sha256`, are on every [release](https://github.com/costajohnt/groovyfmt/releases). See also [Windows](https://github.com/costajohnt/groovyfmt/blob/main/docs/windows.md). Not published yet: the Gradle plugin on the Gradle Plugin Portal (it works from a checkout) and a Docker image.

## Usage

| Command | Does |
|---|---|
| `groovyfmt` | format files under the current directory in place |
| `groovyfmt --check` | list files that would change, exit 1 if any (`--diff` to show them) |
| `groovyfmt --lint` | report lint violations, exit 1 if any |
| `groovyfmt --lint --fix` | fix what can be fixed (imports, semicolons), report the rest |
| `groovyfmt --syntax` | report parse errors |
| `groovyfmt --explain <Rule>` | explain a lint rule |
| `groovyfmt --suggest-style` | suggest indent settings that fit existing code |

Exit status: 0 clean, 1 something to fix, 2 error. Picks up `*.groovy`, `*.gradle`, Jenkinsfiles and groovy `#!` scripts. [Command line](https://github.com/costajohnt/groovyfmt/blob/main/docs/usage.md) has every option.

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
```

`// groovyfmt: off` / `// groovyfmt: on` protect a region. More in [Formatting](https://github.com/costajohnt/groovyfmt/blob/main/docs/formatting.md) and [Configuration](https://github.com/costajohnt/groovyfmt/blob/main/docs/configuration.md).

## Linting

82 rules, 58 on by default (`groovyfmt --rules`). General rules follow [CodeNarc](https://codenarc.org/) names; Jenkinsfile, Gradle and Spock rules catch mistakes that otherwise surface only when a build runs. Coming from CodeNarc or npm-groovy-lint? `groovyfmt --import-codenarc <config>` converts your config. Suppress with `// groovyfmt-ignore` or `@SuppressWarnings`; adopt gradually with `--write-baseline`.

[Lint](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md) has every rule. [Output formats](https://github.com/costajohnt/groovyfmt/blob/main/docs/output-formats.md) covers JSON, SARIF and GitHub annotations.

## CI

```yaml
- uses: actions/checkout@v5
- uses: costajohnt/groovyfmt@v0.27.0      # runs groovyfmt --check --output-format github
- run: groovyfmt --lint --output-format github
```

pre-commit:

```yaml
repos:
  - repo: https://github.com/costajohnt/groovyfmt
    rev: v0.27.0
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
