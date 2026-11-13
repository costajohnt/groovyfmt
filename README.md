# groovyfmt

A formatter and linter for Groovy, Gradle build scripts (`build.gradle`) and Jenkinsfiles. It is one static binary with no JVM, written in Rust on a [tree-sitter](https://tree-sitter.github.io/) grammar.

Run on [jenkins-infra/pipeline-library](https://github.com/jenkins-infra/pipeline-library):

```console
$ groovyfmt --check
Jenkinsfile
test/groovy/BaseTest.groovy
test/groovy/BuildDockerAndPublishImageStepTests.groovy
...
would reformat 27 of 27 files
$ groovyfmt
reformatted 27 of 27 files
$ groovyfmt --lint
test/groovy/BaseTest.groovy:2:1: UnusedImport: The [mock.CurrentBuild] import is never referenced (fixable)
test/groovy/BaseTest.groovy:4:1: UnusedImport: The [org.junit.Before] import is never referenced (fixable)
...
10 violations in 27 files (UnnecessaryGroovyImport 1, UnusedImport 9)
```

## Why it is safe to run

- **Whitespace only.** By default formatting changes indentation, trailing whitespace, blank lines and the final newline. Two opt-in settings fix spacing within lines and wrap long lines.
- **Token check.** The output is parsed again and its tokens compared with the input's. If anything other than whitespace would change, the file is left untouched.
- **Idempotent.** Running it twice changes nothing the second time.
- **Parse first.** A file it can't parse is never formatted, linted or fixed. On a corpus of 23,766 files, 99.6% parse ([coverage](#coverage)).

## Install

Homebrew (macOS and Linux, arm64 and x86_64):

```sh
brew install costajohnt/tap/groovyfmt
```

npm, for Linux and macOS on x64 and arm64 and for Windows x64. The package downloads only the binary for the platform it runs on.

```sh
npx groovyfmt --check                 # run without installing
npm install --save-dev groovyfmt      # or add it to a project
```

Cargo, with Rust 1.90+ and a C compiler. The grammar is vendored in the crate, so nothing beyond crates.io is fetched.

```sh
cargo install --locked groovyfmt
```

Release binaries for Linux (x86_64 and arm64, static musl builds that need no glibc), macOS (Apple Silicon and Intel) and Windows (x86_64, a `.zip`) are on each [release](https://github.com/costajohnt/groovyfmt/releases), each with a `.sha256` file:

```sh
gh release download --repo costajohnt/groovyfmt --pattern '*aarch64-apple-darwin.tar.gz*'
shasum -a 256 -c groovyfmt-*.tar.gz.sha256
tar -xzf groovyfmt-*.tar.gz && mv groovyfmt-*/groovyfmt /usr/local/bin/
```

On Windows, see [Windows](https://github.com/costajohnt/groovyfmt/blob/main/docs/windows.md). The latest source builds with `cargo install --locked --git https://github.com/costajohnt/groovyfmt`.

Also available: a [GitHub Action and pre-commit hooks](#ci-and-pre-commit). Not published yet: the Gradle plugin on the Gradle Plugin Portal (it works from a checkout) and a Docker image.

## Quick start

| Command | Does |
|---|---|
| `groovyfmt` | format the files under the current directory in place |
| `groovyfmt --check` | list files that would be reformatted, exit 1 if any (add `--diff` to see the changes) |
| `groovyfmt --lint` | report lint violations, exit 1 if any |
| `groovyfmt --lint --fix` | remove unused, duplicate and same-package imports and unnecessary semicolons, then report the rest |
| `groovyfmt --syntax` | report each place a file could not be parsed |
| `groovyfmt --explain <Rule>` | explain a lint rule and how to turn it off |
| `groovyfmt --suggest-style` | suggest the indent settings that fit the existing code |

Exit status is 0 when clean, 1 when there is something to fix, and 2 on errors. Paths can be files or directories. Directories are searched for `*.groovy`, `*.gradle`, `*.gvy`, `*.gy`, `*.gsh` and `Jenkinsfile`.

[Command line](https://github.com/costajohnt/groovyfmt/blob/main/docs/usage.md) has every option, which files are picked up, `--lines` and `--changed-since`.

## Formatting

Every line is re-indented from the parse tree. Lines inside multi-line strings and comments are left as they are. Settings go in `.groovyfmt.toml`, found per file by walking up from its directory:

```toml
indent = 4                  # spaces per block level
continuation_indent = 8     # default: same as indent
use_tabs = false
normalize_spacing = false   # true: also fix the spaces between tokens within a line
max_line_length = 120       # wrap longer lines where that is safe (default: no wrapping)
exclude = ["generated/**"]
```

`// groovyfmt: off` and `// groovyfmt: on` leave the lines between them as written.

- [Formatting](https://github.com/costajohnt/groovyfmt/blob/main/docs/formatting.md): what changes, spacing rules, where lines wrap and where they never do.
- [Configuration](https://github.com/costajohnt/groovyfmt/blob/main/docs/configuration.md): every key, config discovery, `exclude`, and `--suggest-style`.

## Linting

`groovyfmt --lint` has 67 rules, 54 of them on by default. The general rules are named after their [CodeNarc](https://codenarc.org/) equivalents. The Jenkinsfile, Gradle and Spock rules catch mistakes those tools otherwise report only when a build runs. `groovyfmt --rules` lists them all.

- Turn rules on or off in the `[lint]` table of `.groovyfmt.toml`.
- Suppress a report with `// groovyfmt-ignore`. CodeNarc's `codenarc-disable` comments and `@SuppressWarnings` work too.
- Adopt lint in an existing repository with a baseline (`--write-baseline`), so only new violations are reported.

[Lint](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md) has every rule, fixing, suppressing and the baseline. [Output formats](https://github.com/costajohnt/groovyfmt/blob/main/docs/output-formats.md) covers JSON, SARIF and GitHub annotations. [What the Gradle rules cover](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md#what-the-gradle-rules-cover) explains what they can check without running Gradle and what requires a Gradle-integrated linter.

## CI and pre-commit

GitHub Actions:

```yaml
- uses: actions/checkout@v5
- uses: costajohnt/groovyfmt@v0.12.0      # runs groovyfmt --check --output-format github
- run: groovyfmt --lint --output-format github
```

pre-commit:

```yaml
repos:
  - repo: https://github.com/costajohnt/groovyfmt
    rev: v0.12.0
    hooks:
      - id: groovyfmt        # format in place; or groovyfmt-check to only report
      - id: groovyfmt-lint
```

Add `--strict` in CI to fail when a file was skipped (it doesn't parse, isn't UTF-8 or is over 10 MB).

[CI](https://github.com/costajohnt/groovyfmt/blob/main/docs/ci.md) has the action's inputs, SARIF upload, the Gradle plugin and setups without either.

## Editors

`groovyfmt --lsp` is a language server for formatting (whole file and selection), parse errors and lint violations as you type, and lint fixes as code actions. It has no completion or navigation. Editors without a language server client can use `groovyfmt --stdin --stdin-filepath <path>`.

For VS Code, the groovyfmt extension ([`editors/vscode`](https://github.com/costajohnt/groovyfmt/tree/main/editors/vscode)) starts the language server with no setup and downloads groovyfmt if it isn't installed. [Editors](https://github.com/costajohnt/groovyfmt/blob/main/docs/editors.md) has setup for Neovim, VS Code, Helix, Zed and IntelliJ IDEA.

## Coding agents

A coding agent that edits a Jenkinsfile or Gradle script has no compiler to tell it whether the file still parses. `groovyfmt --syntax <file>` answers in milliseconds with `file:line:col` messages, and `--output-format json` gives a machine-readable report.

[Coding agents](https://github.com/costajohnt/groovyfmt/blob/main/docs/agents.md) has a block to paste into `AGENTS.md` or `CLAUDE.md` and a Claude Code hook that formats and lints every Groovy file the agent edits.

## Speed

Against [npm-groovy-lint](https://github.com/nvuillam/npm-groovy-lint) 19.0.1 (CodeNarc 3.7.0) on the [spock](https://github.com/spockframework/spock) repository, 635 files, 4-core Linux box:

| task | npm-groovy-lint | groovyfmt |
|---|---|---|
| lint, the same four rules | 19.5 s | 0.11 s (all 14 rules then on by default) |
| lint, npm-groovy-lint's default rule set | 30.3 s | |
| format | did not finish in 15 min (two attempts) | 0.19 s |

The groovyfmt figures were measured on 2026-11-10 with a build between 0.5.0 and 0.6.0, when 14 rules were on by default. Today 54 are.

The comparison flatters groovyfmt somewhat. It skips the 9 spock files it can't parse, which npm-groovy-lint still processes, and CodeNarc's default rule set is far larger. All 9 are generated AST snapshots used as test fixtures, and 8 of them aren't valid Groovy source.

## Coverage

On a corpus of 23,766 files from grails-core, groovy, gradle, spock, CodeNarc, nextflow and Jenkins pipeline repositories, 99.6% parse cleanly and 85 are skipped. Of those 85, 80 aren't Groovy source (templates and test fixtures). On [OpenBoxes](https://github.com/openboxes/openboxes), a production Grails application outside the corpus, 99.5% of its 1,327 files parse.

[Coverage](https://github.com/costajohnt/groovyfmt/blob/main/docs/coverage.md) has the breakdown and the grammar's history.

## Development

```sh
scripts/ci-local.sh [corpus-dir]         # everything CI checks on Linux; run it before merging
cargo test                               # unit and end-to-end CLI tests
cargo clippy --all-targets -- -D warnings
groovyfmt --stats <dir>                  # corpus report, including an idempotence check
groovyfmt --errors <dir>                 # first parse error per failing file, for grammar work
groovyfmt --tree <file>                  # dump the parse tree
```

The grammar in `grammar/` is a copy, so don't edit it here. To change it:

1. Fix `grammar.js` or `src/scanner.c` in a [tree-sitter-groovy](https://github.com/costajohnt/tree-sitter-groovy) checkout.
2. Run `tree-sitter generate` and its tests there, and commit.
3. Run `scripts/update-grammar.sh <checkout> [rev]` here. The checkout must be clean, and `rev` defaults to its HEAD.
4. Compare `--stats` and `--lint` on the corpus before and after.

Releases: push a `v*` tag on `main` matching the `Cargo.toml` version and with a CHANGELOG heading. The release workflow publishes only after those checks, fmt, clippy, the tests on stable and Rust 1.90, and a build and smoke test of all five release binaries pass. [Releasing](https://github.com/costajohnt/groovyfmt/blob/main/docs/releasing.md) has the details.

## License

MIT
