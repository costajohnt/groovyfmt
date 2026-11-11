# groovyfmt

A fast formatter and linter for Groovy, Gradle build scripts (`build.gradle`), and Jenkinsfiles. One static binary, no JVM, written in Rust on a [tree-sitter](https://tree-sitter.github.io/) grammar.

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
test/groovy/BaseTest.groovy:2:1: UnusedImport: The [mock.CurrentBuild] import is never referenced
test/groovy/BaseTest.groovy:4:1: UnusedImport: The [org.junit.Before] import is never referenced
...
9 violations in 27 files (UnusedImport 9)
```

## Speed

Against [npm-groovy-lint](https://github.com/nvuillam/npm-groovy-lint) 19.0.1 (CodeNarc 3.7.0) on the [spock](https://github.com/spockframework/spock) repository, 635 files, 4-core Linux box:

| task | npm-groovy-lint | groovyfmt |
|---|---|---|
| lint, the same four rules | 19.5 s | 0.11 s (all 14 default rules) |
| lint, npm-groovy-lint's default rule set | 30.3 s | |
| format | did not finish in 15 min (two attempts) | 0.19 s |

The comparison flatters groovyfmt somewhat. It skips the 9 spock files it can't parse, which npm-groovy-lint still processes, and CodeNarc's default rule set is far larger. All 9 are generated AST snapshots used as test fixtures, and 8 of them aren't valid Groovy source.

## Install

| Channel | Status |
|---|---|
| [Release binaries](#release-binaries) | available |
| [Build from source](#build-from-source) | available |
| [pre-commit](#pre-commit) | available from `main`, and in releases after 0.8.0 |
| [GitHub Action](#github-actions) | available from `main`, and in releases after 0.8.0; while the repository is private, only with a token that can read it |
| [npm](#npm) (`npx groovyfmt`) | not published yet; comes once the repository is public |
| [Homebrew](#homebrew) | not published yet; comes once the repository is public |
| [Docker image](#docker) | not published yet; comes once the repository is public |

### Release binaries

Prebuilt binaries for Linux (x86_64 and arm64, both static musl builds that need no glibc) and macOS (Apple Silicon, Intel) are attached to each [release](https://github.com/costajohnt/groovyfmt/releases), each with a `.sha256` file:

```sh
gh release download --repo costajohnt/groovyfmt --pattern '*aarch64-apple-darwin.tar.gz*'
shasum -a 256 -c groovyfmt-*.tar.gz.sha256
tar -xzf groovyfmt-*.tar.gz && mv groovyfmt-*/groovyfmt /usr/local/bin/
```

### Build from source

Rust 1.90+ and a C compiler:

```sh
cargo install --locked --git https://github.com/costajohnt/groovyfmt
```

### npm

Not published yet. Once it is, `npx groovyfmt` runs it without installing, and `npm install --save-dev groovyfmt` adds it to a project. The package downloads only the binary for your platform (Linux or macOS, x64 or arm64).

### Homebrew

Not published yet. Once the tap exists:

```sh
brew install costajohnt/tap/groovyfmt
```

### Docker

Not published yet. Once it is, the image holds only the static binary and works on the mounted directory:

```sh
docker run --rm -v "$PWD:/work" ghcr.io/costajohnt/groovyfmt --check
```

## Usage

```text
groovyfmt [OPTIONS] [PATH]...
```

With no paths it works on the current directory. Directories are searched for `*.groovy`, `*.gradle`, `*.gvy`, `*.gy`, `*.gsh` and `Jenkinsfile`, skipping hidden directories, `build`, `out`, `target`, `node_modules` and anything matching the config's [`exclude`](#configuration) patterns. Symlinks inside those directories are not followed. A symlink named on the command line is followed: a linked file is formatted and the link stays a link, and a linked directory is searched. A named link (to a file or a directory) whose target is outside the working directory is read by `--check` and `--lint` but nothing is rewritten through it; formatting it in place is an error. This applies only when the argument itself is a link: `../other-project` or an absolute path is formatted as usual. A file named on the command line is processed even inside a skipped directory, unless it or a directory above it matches `exclude`. Files that aren't valid UTF-8, aren't regular files, are larger than 10 MB, or would need more than 200 columns of indentation (50 levels at the default indent) are skipped and listed under `-v` or `--strict`.

| Command | Does |
|---|---|
| `groovyfmt` | format files in place |
| `groovyfmt --check` | list files that would be reformatted, exit 1 if any |
| `groovyfmt --lint` | report lint violations, exit 1 if any |
| `groovyfmt --syntax` | only parse, and report each place groovyfmt could not parse, exit 1 if any |
| `groovyfmt --stdin` | format stdin to stdout, for editors |
| `groovyfmt --rules` | list lint rules and whether each is on |
| `groovyfmt --suggest-style` | suggest the indent settings that fit the code, writing nothing (see [Configuration](#configuration)) |

Options: `--config <file>`, `--indent <n>`, `--stdin-filepath <path>` (see [Editors](#editors)), `--output-format text|github`, `--strict` (list skipped files and exit 1 if there are any, with `--check`, `--lint` or in place), `-v` (also list skipped files and timing), and `--` to treat every later argument as a path. Exit status is 0 when clean, 1 when files need formatting or lint found something (or, with `--strict`, files were skipped), and 2 on errors (bad arguments, bad config, unreadable files, or `--stdin` input that can't be formatted).

### Parse errors

A file groovyfmt can't parse is never formatted or linted. `--syntax` lists where parsing failed, one line per place, up to 10 per file and then a count of the rest; `-v` and `--strict` show the same lines under each skipped file, and `--stdin` prints them after its error:

```console
$ groovyfmt --syntax
Jenkinsfile:4:14: could not parse: `{` is never closed in a closure
src/Build.groovy:12:21: could not parse: expected `)` to close the `(` on line 12
could not parse 2 of 31 files
```

Columns count characters. The message names the bracket left open when there is one, else quotes the text the grammar stopped at and says where it was (in an argument list, at the top level). It says groovyfmt could not parse the code, not that the code is wrong: the grammar is not the Groovy compiler, and about 0.5% of valid files don't parse (see [Coverage](#coverage)). Exit status is 0 when every file parses, 1 when one doesn't, 2 on errors; `--output-format github` turns each place into an annotation.

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
| UnusedImport | on | an import whose name (or alias) never appears in code or as a word in a comment |
| DuplicateImport | on | the same import twice |
| UnnecessarySemicolon | on | a `;` at the end of a line, unless the next line starts with `{` (without the `;` that closure would become an argument of the statement before) |
| EmptyCatchBlock | on | an empty `catch`, unless the parameter is named `ignore` or `ignored` |
| EmptyIfStatement | on | an `if` with an empty block |
| EmptyElseBlock | on | an `else` with an empty block |
| EmptyTryBlock | on | a `try` with an empty block |
| EmptyFinallyBlock | on | a `finally` with an empty block |
| EmptyWhileStatement | on | a `while` loop with an empty block |
| EmptyForStatement | on | a `for` loop with an empty block |
| DuplicateMapKey | on | a map literal with the same constant key twice (`a:` and `'a':` count as the same); the later entry wins |
| DuplicateCaseStatement | on | two `case` labels for the same constant (`'a'` and `"a"` count as the same) |
| AssignmentInConditional | on | `if (x = 1)`, where `==` was probably meant; `while (x = next())` is a common idiom and is exempt, and so is an assignment in extra parentheses, `if ((m = re.matcher(s)))` |
| ConstantIfExpression | on | an `if` whose condition is a literal, so one branch never runs |
| UnnecessaryGString | off | a double-quoted string with nothing to interpolate (noisy where double quotes are the house style) |
| ComparisonWithSelf | off | a variable compared with itself (Spock equality checks do this on purpose); not inside a GORM where query (`Book.where { title == title }`, `@Where`), where the left side is the domain property |

A block holding only a comment doesn't count as empty.

## Configuration

Each file is governed by the nearest `.groovyfmt.toml` or `groovyfmt.toml` found by walking up from that file's own directory (with `--stdin-filepath`, from that path's directory), wherever groovyfmt runs from. `groovyfmt --check ../proj` uses `proj`'s config, and one run from a repository root formats a subproject with its own config by that config. A nested config replaces its parent's completely: nothing is merged, so repeat any settings the subproject should keep. The walk doesn't stop at the repository root, so a config in a directory above the checkout (a shared workspace directory, your home directory) applies too; `-v` shows which one is used. Files with no config above them use the defaults. `--config <file>` skips discovery and applies that one file to everything; `--indent` overrides the indent of whichever config applies. `--rules` and `--stdin` without `--stdin-filepath` use the config for the current directory. Everything is optional:

```toml
indent = 4                  # spaces per block level, 1 to 16
continuation_indent = 8     # arguments, literals, continued lines (default: same as indent)
use_tabs = false            # true: indent with tabs, one per indent level
indent_after_labels = false # true: indent Spock when:/then: blocks under their label
exclude = ["generated/**", "**/*.gsh"] # skip these files and directories

[lint]                      # rule = true/false; unlisted rules keep their default
UnnecessaryGString = true
UnusedImport = false
```

`exclude` patterns are relative to the directory holding the config file, whatever directory groovyfmt runs from. They match whole paths: `*` and `?` stay within one path segment, `**` spans any number of segments, and a matching directory is skipped with everything under it, including any config inside it. A nested config's `exclude` applies only inside its own directory, and whether that directory is searched at all is up to the config above it. That holds for files named on the command line, `--stdin-filepath`, and runs from inside the excluded directory too; pass `--config` to format such a directory anyway. So `vendor` skips only the top-level `vendor`, `**/vendor` skips one at any depth, and `src/*.groovy` doesn't reach into `src/main/`. Excludes also apply to files named on the command line (pre-commit passes them that way) and to `--stdin-filepath`.

Unknown keys and misspelled rule names are errors, so a typo can't silently do nothing; a config file can only fail the run when groovyfmt reaches a file or directory it governs. `-v` prints each config in use.

`continuation_indent = 8` matches IntelliJ's default and most Grails projects written in it. On OpenBoxes it cuts the first-run diff from 723 files to 454, and what remains is mostly trailing whitespace and runs of blank lines.

`use_tabs = true` writes leading indentation as tabs. Indents are worked out in columns as usual, with a tab `indent` columns wide, and each full tab width becomes a tab. So with the default `continuation_indent` (the same as `indent`) a continued line gets one more tab than the line it continues, and `continuation_indent = 8` with `indent = 4` gives it two. A `continuation_indent` that isn't a multiple of `indent` leaves the remainder as spaces after the tabs. On spring-boot, which indents with tabs, `use_tabs = true` cuts the first-run diff from 762 files to 95, and what remains is mostly missing final newlines, runs of blank lines and a few files that are indented with spaces.

To find the settings that fit existing code before the first run, use `groovyfmt --suggest-style [PATH]...`. It formats every file in memory, writing nothing, under six styles: the defaults, `indent = 2`, `use_tabs = true`, `continuation_indent = 8`, `indent = 2` with `continuation_indent = 4`, and `use_tabs = true` with `continuation_indent = 8`. It prints the config lines for the style that leaves the most files unchanged on stdout, ready to paste into `.groovyfmt.toml` in place of any indent settings already there, and a table of how many files and lines each style would change on stderr. A tie goes to the style that changes fewer lines, then to the defaults, then to the style with fewer settings. Everything else in the config applies as usual (`exclude`, `indent_after_labels`), `--indent` can't be combined with it, and it exits 2 when there are no files or none of them can be formatted.

```console
$ groovyfmt --suggest-style
  style                                        files   lines
  defaults                                       445   33891
* indent = 2                                     113    8036
  use_tabs = true                                448   34175
  continuation_indent = 8                        445   33898
  indent = 2, continuation_indent = 4            176    9050
  continuation_indent = 8, use_tabs = true       448   34175
files: files changed, of 454 (4 skipped under every style); lines: lines changed; in 0.82s
# suggested by groovyfmt --suggest-style: 337 of 454 files unchanged
indent = 2
```

Spock specs come in two styles. By default the statements of a `given:` / `when:` / `then:` block stay level with the label, as in Spock's documentation; `indent_after_labels = true` puts them one level deeper. In grails-core about half the specs use each style, so pick the one your project already uses before the first run.

## CI

### GitHub Actions

The action installs the release binary for the runner (Linux or macOS, x64 or arm64), checks it against its `.sha256`, puts it on `PATH` and runs it with `args`:

```yaml
- uses: actions/checkout@v5
- uses: costajohnt/groovyfmt@main  # or a release tag after v0.8.0
  # with:
  #   args: --check --output-format github   # the default; "" only installs
  #   version: 0.8.0                         # default: the release of the action ref used
  #   token: ${{ github.token }}             # the default
- run: groovyfmt --lint --output-format github
```

`--output-format github` turns findings into annotations on the pull request diff. While the repository is private, `token` must be able to read it, which `github.token` can only do in workflows of this repository. Without `version`, the action installs the release named by the `Cargo.toml` at the ref you use, so a release tag installs that release and `main` the latest one.

Without the action:

```yaml
- name: groovyfmt
  env:
    GH_TOKEN: ${{ github.token }}
  run: |
    gh release download --repo costajohnt/groovyfmt --pattern '*x86_64-unknown-linux-musl.tar.gz*'
    shasum -a 256 -c groovyfmt-*.tar.gz.sha256
    tar -xzf groovyfmt-*.tar.gz && mv groovyfmt-*/groovyfmt .
    ./groovyfmt --check --output-format github
    ./groovyfmt --lint --output-format github
```

This snippet has the same token limit: outside this repository, set `GH_TOKEN` to a token that can read `costajohnt/groovyfmt` (for example a fine-grained personal access token with read access to its contents, stored as a secret).

Add `--strict` to `--check` or `--lint` to fail the job when a file was skipped (it doesn't parse, isn't UTF-8 or is over 10 MB), which otherwise passes unchecked.

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
  - repo: https://github.com/costajohnt/groovyfmt
    rev: main  # or a release tag after v0.8.0, which pre-commit prefers
    hooks:
      - id: groovyfmt        # format in place; or groovyfmt-check to only report
      - id: groovyfmt-lint
```

These hooks run the `groovyfmt` on `PATH`. To have pre-commit build it from source instead (Rust 1.90+ and a C compiler, compiled once per `rev`), use `groovyfmt-source`, `groovyfmt-check-source` and `groovyfmt-lint-source`. While the repository is private, pre-commit needs git credentials that can clone it. Or point a local hook at the binary on `PATH`:

```yaml
repos:
  - repo: local
    hooks:
      - id: groovyfmt
        name: groovyfmt
        entry: groovyfmt
        language: system
        files: (\.(groovy|gradle|gvy|gy|gsh)|(^|/)Jenkinsfile)$
```

pre-commit passes the staged files by name, and `exclude` in `.groovyfmt.toml` still applies to them, so generated code stays untouched without repeating the patterns in the hook.

## Editors

Neovim and VS Code call `groovyfmt --stdin`, which prints nothing and exits 2 when it can't format the buffer, so a parse error never replaces your code.

Add `--stdin-filepath <path>` (it requires `--stdin`) to tell groovyfmt which file the buffer belongs to. The config is then found by walking up from that file's directory, as for a file named on the command line, instead of from the working directory; the path names the buffer in error messages, and if the path matches `exclude` the buffer comes back unchanged with exit 0. The file doesn't have to exist yet.

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

groovyfmt only touches files it can parse. On a corpus of 23,766 files from grails-core, groovy, gradle, spock, CodeNarc, nextflow and Jenkins pipeline repositories, 99.5% parse cleanly. On [OpenBoxes](https://github.com/openboxes/openboxes), a production Grails application that isn't in the corpus, 99.5% of its 1,327 files do.

The grammar is [dekobon/tree-sitter-groovy](https://github.com/dekobon/tree-sitter-groovy), currently pinned to the fork branch [`costajohnt/tree-sitter-groovy@groovyfmt-base`](https://github.com/costajohnt/tree-sitter-groovy/tree/groovyfmt-base), which adds upstream PR #49 plus our fixes to the 0.3.0 release:

| grammar | files parsing cleanly |
|---|---|
| dekobon 0.3.0 | 63.8% |
| + PR #49 | 75.0% |
| + #56 (`final` locals) and #57 (named-argument calls) | 80.9% |
| + nested type declarations (`static class`, member `interface`, …) | 85.6% |
| + a `[` starting a new line is a list or map, not a subscript (Grails controllers returning `[model: …]`); `final` parameters; traits with `implements` | 88.1% |
| + annotated script and field declarations (`@Field`, `@Shared`); command calls with several arguments, quoted keys or a dotted receiver; array creation; lambdas; nested annotations; multiple assignment; `def static`, `def <T>`, `synchronized` and smaller forms | 96.1% |
| + array types as values (`String[].class`); command calls as declaration and assignment values (`def r = sh returnStdout: true, script: 'x'`); a name or `(` on a new line starts a statement; `record` as a name; annotations after modifiers; statements in GString interpolations; `//` inside GStrings; closure parameter defaults and smaller forms | 98.7% |
| + `var`, `pipeline` and `trait` as names; generic types as values (`Mock(Supplier<Long>)`); command calls with `new`, `null`, `!` or dotted first arguments and in parentheses (`(el x).click()`); anonymous class bodies; `instanceof` pattern variables; slashy strings starting with a space; `\` line continuations; annotated and `final` closure parameters, `default` methods and smaller forms | 99.5% |

Of the 117 corpus files still skipped, 80 aren't Groovy source: 70 Grails, Gradle and CodeNarc templates with `@artifact.package@`, `<% %>` or `${className}` placeholders, 9 Spock AST snapshots and one deliberately broken groovydoc fixture. The other 37 use forms the grammar doesn't parse yet, among them Groovy 5 syntax (`for await`, `case Integer i when`, indexed `for (i, x in xs)`, `??`), slashy strings containing `\\/`, and one-off forms such as nested array initializers and `{a, b}` annotation arrays.

## Development

```sh
scripts/ci-local.sh [corpus-dir]         # everything CI checks on Linux; run it before merging
cargo test                               # unit and end-to-end CLI tests
cargo clippy --all-targets -- -D warnings
groovyfmt --stats <dir>                  # corpus report, including an idempotence check
groovyfmt --errors <dir>                 # first parse error per failing file, for grammar work
groovyfmt --tree <file>                  # dump the parse tree
```

Releases: push a `v*` tag on `main` matching the Cargo.toml version and with a CHANGELOG heading; the release workflow publishes only after those checks, fmt, clippy, the tests on stable and Rust 1.90, and a build and smoke test of all four binaries pass.

## License

MIT
