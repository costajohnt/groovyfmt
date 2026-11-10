# groovyfmt

A fast formatter and linter for Groovy, Gradle build scripts (`build.gradle`), and Jenkinsfiles. One static binary, no JVM, written in Rust on a [tree-sitter](https://tree-sitter.github.io/) grammar.

Run on [jenkins-infra/pipeline-library](https://github.com/jenkins-infra/pipeline-library):

```console
$ groovyfmt --check
Jenkinsfile
test/groovy/BaseTest.groovy
test/groovy/BuildDockerAndPublishImageStepTests.groovy
...
would reformat 21 of 27 files, 6 skipped (-v shows why)
$ groovyfmt
reformatted 21 of 27 files, 6 skipped (-v shows why)
$ groovyfmt --lint
test/groovy/BaseTest.groovy:2:1: UnusedImport: The [mock.CurrentBuild] import is never referenced
test/groovy/BaseTest.groovy:4:1: UnusedImport: The [org.junit.Before] import is never referenced
...
7 violations in 27 files (UnusedImport 7), 6 skipped (-v shows why)
```

## Speed

Against [npm-groovy-lint](https://github.com/nvuillam/npm-groovy-lint) 19.0.1 (CodeNarc 3.7.0) on the [spock](https://github.com/spockframework/spock) repository, 635 files, 4-core Linux box:

| task | npm-groovy-lint | groovyfmt |
|---|---|---|
| lint, the same four rules | 19.5 s | 0.12 s (all 14 default rules) |
| lint, npm-groovy-lint's default rule set | 30.3 s | |
| format | did not finish in 15 min (two attempts) | 0.25 s |

The comparison flatters groovyfmt somewhat. It skips the 119 spock files it can't parse, which npm-groovy-lint still processes, and CodeNarc's default rule set is far larger.

## Install

Prebuilt binaries for Linux (x86_64 static, arm64) and macOS (Apple Silicon, Intel) are attached to each [release](https://github.com/costajohnt/groovyfmt/releases):

```sh
gh release download --repo costajohnt/groovyfmt --pattern '*aarch64-apple-darwin.tar.gz'
tar -xzf groovyfmt-*.tar.gz && mv groovyfmt-*/groovyfmt /usr/local/bin/
```

Or build from source (Rust 1.85+, a C compiler):

```sh
cargo install --locked --git https://github.com/costajohnt/groovyfmt
```

## Usage

```text
groovyfmt [OPTIONS] [PATH]...
```

With no paths it works on the current directory. Directories are searched for `*.groovy`, `*.gradle`, `*.gvy`, `*.gy`, `*.gsh` and `Jenkinsfile`, skipping hidden directories, `build`, `out`, `target`, `node_modules` and anything matching the config's [`exclude`](#configuration) patterns. Symlinks inside those directories are not followed. A symlink named on the command line is followed: a linked file is formatted and the link stays a link, and a linked directory is searched. A named link (to a file or a directory) whose target is outside the working directory is read by `--check` and `--lint` but nothing is rewritten through it; formatting it in place is an error. This applies only when the argument itself is a link: `../other-project` or an absolute path is formatted as usual. A file named on the command line is processed even inside a skipped directory, unless it matches `exclude`. Files that aren't valid UTF-8, aren't regular files, are larger than 10 MB, or would need more than 200 columns of indentation (50 levels at the default indent) are skipped and listed under `-v`.

| Command | Does |
|---|---|
| `groovyfmt` | format files in place |
| `groovyfmt --check` | list files that would be reformatted, exit 1 if any |
| `groovyfmt --lint` | report lint violations, exit 1 if any |
| `groovyfmt --stdin` | format stdin to stdout, for editors |
| `groovyfmt --rules` | list lint rules and whether each is on |

Options: `--config <file>`, `--indent <n>`, `--stdin-filepath <path>` (see [Editors](#editors)), `--output-format text|github`, `-v` (also list skipped files and timing). Exit status is 0 when clean, 1 when files need formatting or lint found something, and 2 on errors (bad arguments, bad config, unreadable files, or `--stdin` input that can't be formatted).

### What formatting changes

Whitespace only:

- every line is re-indented from the parse tree (blocks, closures, argument lists, list and map literals, continued expressions, Spock `when:` / `then:` blocks)
- trailing whitespace is removed, runs of blank lines collapse to one, and the file ends with one newline
- lines inside multi-line strings and comments are left exactly as they are

Formatting is safe by construction. The output is parsed again and its token stream compared with the input's; if anything other than whitespace would change, or the input doesn't parse cleanly, the file is left untouched (`-v` lists these). Running it twice changes nothing the second time.

### Lint rules

Named after their [CodeNarc](https://codenarc.org/) equivalents so existing configurations map across. Files that don't parse are not linted, which keeps false positives out.

| Rule | Default | Flags |
|---|---|---|
| UnusedImport | on | an import whose name (or alias) never appears in code or comments |
| DuplicateImport | on | the same import twice |
| UnnecessarySemicolon | on | a `;` at the end of a line |
| EmptyCatchBlock | on | an empty `catch`, unless the parameter is named `ignore` or `ignored` |
| EmptyIfStatement | on | an `if` with an empty block |
| EmptyElseBlock | on | an `else` with an empty block |
| EmptyTryBlock | on | a `try` with an empty block |
| EmptyFinallyBlock | on | a `finally` with an empty block |
| EmptyWhileStatement | on | a `while` loop with an empty block |
| EmptyForStatement | on | a `for` loop with an empty block |
| DuplicateMapKey | on | a map literal with the same constant key twice (`a:` and `'a':` count as the same); the later entry wins |
| DuplicateCaseStatement | on | two `case` labels for the same constant |
| AssignmentInConditional | on | `if (x = 1)`, where `==` was probably meant; `while (x = next())` is a common idiom and is exempt |
| ConstantIfExpression | on | an `if` whose condition is a literal, so one branch never runs |
| UnnecessaryGString | off | a double-quoted string with nothing to interpolate (noisy where double quotes are the house style) |
| ComparisonWithSelf | off | a variable compared with itself (GORM `where { title == title }` and Spock equality checks do this on purpose) |

A block holding only a comment doesn't count as empty.

## Configuration

groovyfmt looks for `.groovyfmt.toml` or `groovyfmt.toml` in the current directory and its parents (with `--stdin-filepath`, that file's directory and its parents). Everything is optional:

```toml
indent = 4                  # spaces per block level, 1 to 16
continuation_indent = 8     # arguments, literals, continued lines (default: same as indent)
indent_after_labels = false # true: indent Spock when:/then: blocks under their label
exclude = ["generated/**", "**/*.gsh"] # skip these files and directories

[lint]                      # rule = true/false; unlisted rules keep their default
UnnecessaryGString = true
UnusedImport = false
```

`exclude` patterns are relative to the directory holding the config file, whatever directory groovyfmt runs from. They match whole paths: `*` and `?` stay within one path segment, `**` spans any number of segments, and a matching directory is skipped with everything under it. So `vendor` skips only the top-level `vendor`, `**/vendor` skips one at any depth, and `src/*.groovy` doesn't reach into `src/main/`. Excludes also apply to files named on the command line (pre-commit passes them that way) and to `--stdin-filepath`.

Unknown keys and misspelled rule names are errors, so a typo can't silently do nothing. `--indent` overrides the file.

`continuation_indent = 8` matches IntelliJ's default and most Grails projects written in it. On OpenBoxes it cuts the first-run diff from 694 files to 426, and what remains is mostly trailing whitespace and runs of blank lines.

Spock specs come in two styles. By default the statements of a `given:` / `when:` / `then:` block stay level with the label, as in Spock's documentation; `indent_after_labels = true` puts them one level deeper. In grails-core about half the specs use each style, so pick the one your project already uses before the first run.

## CI

### GitHub Actions

```yaml
- name: groovyfmt
  env:
    GH_TOKEN: ${{ github.token }}
  run: |
    gh release download --repo costajohnt/groovyfmt --pattern '*x86_64-unknown-linux-musl.tar.gz' --output groovyfmt.tgz
    tar -xzf groovyfmt.tgz --strip-components=1
    ./groovyfmt --check --output-format github
    ./groovyfmt --lint --output-format github
```

`--output-format github` turns findings into annotations on the pull request diff. While the repository is private, the token needs read access to it.

### Gradle

```groovy
tasks.register('groovyfmtCheck', Exec) {
    group = 'verification'
    commandLine 'groovyfmt', '--check'
}
tasks.register('groovyfmtLint', Exec) {
    group = 'verification'
    commandLine 'groovyfmt', '--lint'
}
check.dependsOn 'groovyfmtCheck', 'groovyfmtLint'
```

### pre-commit

```yaml
repos:
  - repo: local
    hooks:
      - id: groovyfmt
        name: groovyfmt
        entry: groovyfmt
        language: system
        files: (\.(groovy|gradle|gvy|gy|gsh)|Jenkinsfile)$
```

pre-commit passes the staged files by name, and `exclude` in `.groovyfmt.toml` still applies to them, so generated code stays untouched without repeating the patterns in the hook.

## Editors

All of these call `groovyfmt --stdin`, which prints nothing and exits 2 when it can't format the buffer, so a parse error never replaces your code.

Add `--stdin-filepath <path>` (it requires `--stdin`) to tell groovyfmt which file the buffer belongs to. The config is then found by walking up from that file's directory instead of the working directory, the path names the buffer in error messages, and if the path matches `exclude` the buffer comes back unchanged with exit 0. The file doesn't have to exist yet.

- **Neovim** ([conform.nvim](https://github.com/stevearc/conform.nvim)):
  ```lua
  require("conform").setup({
    formatters = {
      groovyfmt = { command = "groovyfmt", args = { "--stdin", "--stdin-filepath", "$FILENAME" } },
    },
    formatters_by_ft = { groovy = { "groovyfmt" } },
  })
  ```
- **VS Code** ([Custom Local Formatters](https://marketplace.visualstudio.com/items?itemName=jkillian.custom-local-formatters)):
  ```json
  "customLocalFormatters.formatters": [
    { "command": "groovyfmt --stdin", "languages": ["groovy"] }
  ]
  ```
- **IntelliJ IDEA**: Settings, Tools, File Watchers, add a watcher for Groovy files with program `groovyfmt` and arguments `$FilePath$`. To format on demand instead, add an External Tool (Settings, Tools, External Tools) with program `groovyfmt`, arguments `$FilePath$` and working directory `$ProjectFileDir$`, then bind it to a key. Either way the file is named explicitly, so `exclude` patterns are honored.

## Coverage

groovyfmt only touches files it can parse. On a corpus of 23,766 files from grails-core, groovy, gradle, spock, CodeNarc, nextflow and Jenkins pipeline repositories, 88.1% parse cleanly. On [OpenBoxes](https://github.com/openboxes/openboxes), a production Grails application that isn't in the corpus, 95.6% of its 1,327 files do.

The grammar is [dekobon/tree-sitter-groovy](https://github.com/dekobon/tree-sitter-groovy), currently pinned to the fork branch [`costajohnt/tree-sitter-groovy@groovyfmt-base`](https://github.com/costajohnt/tree-sitter-groovy/tree/groovyfmt-base), which adds upstream PR #49 plus our fixes to the 0.3.0 release:

| grammar | files parsing cleanly |
|---|---|
| dekobon 0.3.0 | 63.8% |
| + PR #49 | 75.0% |
| + #56 (`final` locals) and #57 (named-argument calls) | 80.9% |
| + nested type declarations (`static class`, member `interface`, …) | 85.6% |
| + a `[` starting a new line is a list or map, not a subscript (Grails controllers returning `[model: …]`); `final` parameters; traits with `implements` | 88.1% |

## Development

```sh
cargo test                               # unit and end-to-end CLI tests
cargo clippy --all-targets -- -D warnings
groovyfmt --stats <dir>                  # corpus report, including an idempotence check
groovyfmt --errors <dir>                 # first parse error per failing file, for grammar work
groovyfmt --tree <file>                  # dump the parse tree
```

Releases: push a `v*` tag and the release workflow builds and attaches the binaries.

## License

MIT
