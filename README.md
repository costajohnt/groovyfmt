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
test/groovy/BaseTest.groovy:2:1: UnusedImport: The [mock.CurrentBuild] import is never referenced (fixable)
test/groovy/BaseTest.groovy:4:1: UnusedImport: The [org.junit.Before] import is never referenced (fixable)
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
| [Gradle plugin](#gradle) | not published yet; works today from a checkout with `includeBuild` |
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

The Groovy grammar is vendored in the repository (`grammar/`), so this fetches only groovyfmt and its crates.io dependencies.

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
| `groovyfmt --check --diff` | print a unified diff of what formatting would change, exit 1 if any |
| `groovyfmt --lint` | report lint violations, exit 1 if any |
| `groovyfmt --lint --fix` | fix what can be fixed safely (unused and duplicate imports, unnecessary semicolons), then report the rest |
| `groovyfmt --syntax` | only parse, and report each place groovyfmt could not parse, exit 1 if any |
| `groovyfmt --stdin` | format stdin to stdout, for editors |
| `groovyfmt --rules` | list lint rules and whether each is on |
| `groovyfmt --suggest-style` | suggest the indent settings that fit the code, writing nothing (see [Configuration](#configuration)) |

Options: `--config <file>`, `--indent <n>`, `--stdin-filepath <path>` (see [Editors](#editors)), `--output-format text|github|json|sarif` (see [Machine-readable output](#machine-readable-output)), `--strict` (list skipped files and exit 1 if there are any, with `--check`, `--lint` or in place), `--lines <start>-<end>` and `--changed-since <rev>` (see [Changing less](#changing-less)), `-v` (also list skipped files and timing), and `--` to treat every later argument as a path. Exit status is 0 when clean, 1 when files need formatting or lint found something (or, with `--strict`, files were skipped), and 2 on errors (bad arguments, bad config, unreadable files, or `--stdin` input that can't be formatted).

### Parse errors

A file groovyfmt can't parse is never formatted or linted. `--syntax` lists where parsing failed, one line per place, up to 10 per file and then a count of the rest; `-v` and `--strict` show the same lines under each skipped file, and `--stdin` prints them after its error:

```console
$ groovyfmt --syntax
Jenkinsfile:4:14: could not parse: `{` is never closed in a closure
src/Build.groovy:12:21: could not parse: expected `)` to close the `(` on line 12
could not parse 2 of 31 files
```

Columns count characters. The message names the bracket left open when there is one, else quotes the text the grammar stopped at and says where it was (in an argument list, at the top level). It says groovyfmt could not parse the code, not that the code is wrong: the grammar is not the Groovy compiler, and about 0.5% of valid files don't parse (see [Coverage](#coverage)). Exit status is 0 when every file parses, 1 when one doesn't, 2 on errors; `--output-format github` turns each place into an annotation.

### Machine-readable output

`--output-format json` works with `--check`, `--lint`, `--syntax` and formatting in place. stdout then holds one JSON document and nothing else, one line per file; the summary line is left out and so are the `-v` / `--strict` lists of skipped files, which the document already has. stderr still gets errors and `-v`'s `config:` lines. Exit statuses are unchanged; exit 2 with an empty stdout is a usage or config error.

```console
$ groovyfmt --lint --output-format json
{"version":1,"files":[
{"path":"A.groovy","status":"unchanged","skip_reason":null,"diagnostics":[{"rule":"UnusedImport","severity":"warning","message":"The [a.b.Unused] import is never referenced","line":1,"column":1,"end_line":1,"end_column":18,"fixable":true}]},
{"path":"Broken.groovy","status":"skipped","skip_reason":"parse error","diagnostics":[{"rule":"ParseError","severity":"warning","message":"expected `}` to close the `{` on line 1","line":1,"column":10,"end_line":1,"end_column":10,"fixable":false}]}
],"summary":{"files":2,"unchanged":1,"would_reformat":0,"reformatted":0,"skipped":1,"errors":0,"diagnostics":2}}
```

The schema is versioned: `version` changes whenever a field is removed or changes meaning (new fields may be added within a version).

| Field | Meaning |
|---|---|
| `version` | `1` |
| `files[].path` | the file as the text output names it, relative to where groovyfmt runs when it was found by a walk from there (a name that isn't UTF-8 is shown with replacement characters) |
| `files[].status` | `unchanged`, `would_reformat` (`--check`), `reformatted` (in place), `fixed` (`--lint --fix`), `skipped` (not parsed, not UTF-8, over 10 MB, or not formattable) or `error` (unreadable, or the write failed). `--lint` without `--fix` and `--syntax` never change files, so a file they read is `unchanged` |
| `files[].skip_reason` | for `skipped` and `error`, why (`parse error`, `not valid UTF-8`, an I/O error); with `--fix`, `not fixed: <why>` for a file whose fix failed a check; else `null` |
| `files[].diagnostics[]` | lint violations (`--lint`, enabled rules only) and, for a file that doesn't parse, the places groovyfmt could not parse (rule `ParseError`, in every mode, at most 10 per file) |
| `rule`, `severity`, `message` | the rule name, `warning`, and the message text (the same as the text output, unescaped) |
| `line`, `column`, `end_line`, `end_column` | 1-based; columns count characters; the end is just past the flagged code |
| `fixable` | whether `--lint --fix` would fix it |
| `summary` | counts of files by status (`errors` counts `error`) and of diagnostics |

Strings are standard JSON. Every control character is escaped, and so is a `[` after `##` (as `\u005b`), so file names and quoted source can't act as terminal escapes or GitHub Actions workflow commands when the document is printed in a log; a JSON parser reads back the original text.

`--output-format sarif` (with `--lint` or `--syntax`) prints a SARIF 2.1.0 log of the same diagnostics, with every rule (and `ParseError`) described from the rule table, for GitHub code scanning:

```yaml
- run: groovyfmt --lint --output-format sarif > groovyfmt.sarif || true
- uses: github/codeql-action/upload-sarif@v4
  with:
    sarif_file: groovyfmt.sarif
```

### Changing less

`--check --diff` prints a unified diff of what formatting would change instead of the file names, and writes nothing; `--lint --fix --diff` does the same for the fixes. Exit statuses don't change. The diff names files `a/<path>` and `b/<path>` as the text output names them, so from the same directory `git apply` or `patch -p1` applies it. A diff has to print file content as it is, so for a file whose name holds a control character, `"` or `\`, or whose diff would print a control character other than a tab, `##[`, or a line starting with `::`, a `# groovyfmt: <path>: diff not shown` line takes its place (both tools skip it); that keeps a file name or content from forging a patch header or acting on a terminal or a CI log. The name is checked before it goes into the `---` / `+++` lines, and the patch is checked as a whole. Files with CR LF line endings diff as they are, since that CR only ends a line; a CR anywhere else withholds the diff.

`--lines <start>-<end>` (1-based, inclusive, repeatable) changes only those lines, for one file or `--stdin`, as editors do for a selection; every other line comes out byte for byte as it was. Indentation comes from the whole parse tree, so the whole file is formatted and the original text put back outside the ranges, and the result goes through the same token check as formatting. No line is added or removed: a blank line formatting would drop is emptied instead, so the lines after a range keep their numbers and running it again with the same ranges changes nothing.

`--changed-since <rev>` limits a run (formatting, `--check`, `--lint`, `--syntax`) to files under the working directory that differ from git revision `rev` in the working tree or index, plus untracked files git doesn't ignore. Those files are formatted whole. It runs `git diff --name-only -z --relative <rev> --` and `git ls-files -z --others --exclude-standard`; `rev` can't start with `-`, and a missing git or a directory outside a repository is an error (exit 2).

### What formatting changes

Whitespace only:

- every line is re-indented from the parse tree (blocks, closures, argument lists, list and map literals, continued expressions, Spock `when:` / `then:` blocks)
- trailing whitespace is removed, runs of blank lines collapse to one, and the file ends with one newline
- lines inside multi-line strings and comments are left exactly as they are

Formatting is safe by construction. The output is parsed again and its token stream compared with the input's; if anything other than whitespace would change, or the input doesn't parse cleanly, the file is left untouched (`-v` lists these). Running it twice changes nothing the second time.

### Lint rules

The general rules are named after their [CodeNarc](https://codenarc.org/) equivalents so existing configurations map across; the Jenkins, Gradle and Spock rules catch, in milliseconds, mistakes those tools otherwise report only when a build runs. Files that don't parse are not linted, which keeps false positives out.

#### General

| Rule | Default | Flags |
|---|---|---|
| UnusedImport | on | an import whose name (or alias) never appears in code or as a word in a comment; not an annotated import (`@Library`, `@Grab`), which is there for its annotation |
| DuplicateImport | on | the same import twice |
| UnnecessarySemicolon | on | a `;` at the end of a line, unless the next line starts with `{` (without the `;` that closure would become an argument of the statement before), `++` or `--` |
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

#### Jenkinsfiles

What the Declarative Pipeline plugin rejects when a build starts. They check a top-level `pipeline { }` in a file named `Jenkinsfile`, `Jenkinsfile.*` or `*.jenkinsfile`, or in any file whose first statement it is (after comments and imports); scripted pipelines (`node { }`) are left alone. Directory walks find only files named `Jenkinsfile`, so name other pipeline files on the command line. The section lists follow the [syntax reference](https://www.jenkins.io/doc/book/pipeline/syntax/) and the plugin's own parser where they differ.

| Rule | Default | Flags |
|---|---|---|
| JenkinsMissingSection | on | a `pipeline` without `agent` or `stages`; `stages`, `parallel` or `matrix` without a stage; a stage with none of `steps`, `stages`, `parallel`, `matrix`; an empty `steps`, `post`, `when`, `environment`, `options`, `parameters`, `triggers` or `libraries` |
| JenkinsConflictingSections | on | a section twice in the same block, or a stage with more than one of `steps`, `stages`, `parallel`, `matrix` |
| JenkinsUnknownSection | on | a name that isn't a section where it stands (`steps` directly in `pipeline`, `parameters` in a stage, a misspelled `post` condition, the pre-0.8 `properties` or `notifications`), anything but `stage` in `stages`, and `stage`, `properties` or a `parallel` beside other steps in `steps` |
| JenkinsStatementOutsideSection | on | Groovy code (`def x = 1`, `if`, an expression) directly in `pipeline`, `stages`, a stage, `matrix`, `post` or `when` |
| JenkinsScriptRequired | on | `if`, `for`, `while`, `try`, `switch`, `def` or an assignment in `steps` (or in a block step inside it) without `script { }` |
| JenkinsUnknownWhenCondition | on | a `when` condition that isn't built in (`branch`, `expression`, `allOf` and so on) |
| JenkinsMisplacedOption | on | `parameters` or `triggers` inside `options`, or a pipeline-wide option (`buildDiscarder`, `disableConcurrentBuilds`, `skipStagesAfterUnstable`, ...) in a stage's `options` |
| JenkinsDuplicateStageName | on | two stages with the same name in one `stages` or `parallel` |
| JenkinsStageName | on | a stage with no name, or a name that isn't a plain string (`stage("Deploy ${env}")` is rejected) |
| JenkinsAgentForm | on | `agent` with no value, a bare `agent` other than `any` or `none`, or an `agent { }` block that is empty or names more than one agent type |

Option names and agent types aren't checked against a list: plugins add their own, and any block-scoped step is a valid option.

#### Gradle build scripts

In `*.gradle` files.

| Rule | Default | Flags |
|---|---|---|
| GradleRemovedConfiguration | on | `compile`, `runtime`, `testCompile` or `testRuntime` dependencies, removed in Gradle 7.0 (the message names the replacement); not when the build declares a configuration of that name itself |
| GradleJcenter | on | `jcenter()`, deprecated in Gradle 7.0 and removed in 9.0 |
| GradleLeftShiftTask | on | `task name << { }`, removed in Gradle 5.0 |
| GradleDynamicVersion | off | a `+` or `latest.*` dependency version (off by default: whether to pin is a policy choice) |

#### Spock specifications

In classes whose superclass is `Specification` or ends in `Spec` or `Specification` (a project's base spec). The fixture methods `setup`, `cleanup`, `setupSpec` and `cleanupSpec` are skipped, as Spock ignores labels there.

| Rule | Default | Flags |
|---|---|---|
| SpockBlockOrder | on | blocks in an order Spock refuses to compile (`then:` without `when:`, `when:` not followed by `then:`, `given:` after another block, anything but `and:`, `combined:` or `filter:` after `where:`), and labels that aren't Spock blocks |
| SpockDataTableColumns | on | a `where:` data table row with a different number of columns than the row above |
| SpockEmptyBlock | off | a `then:` or `expect:` block with no conditions, only a description (off by default: Spock accepts it, and Spock's own tests use empty blocks on purpose) |

### Fixing

`groovyfmt --lint --fix` removes, in place, what three rules flag: an `UnusedImport` or `DuplicateImport` line (with its line break, and only when nothing but whitespace or a `;` shares the line) and an `UnnecessarySemicolon` `;`. It then reports the violations that remain and exits 1 if there are any. Plain `--lint` marks these violations with ` (fixable)` (`"fixable": true` in JSON). A rule turned off in the config is neither fixed nor reported.

```console
$ groovyfmt --lint --fix
src/A.groovy:4:1: UnusedImport: The [b.C] import is never referenced
fixed 4 violations in 1 files (DuplicateImport 1, UnnecessarySemicolon 2, UnusedImport 1)
1 violations in 3 files (UnusedImport 1)
```

As with formatting, each fix is checked before the file is written: the fixed file must parse cleanly, its tokens must be the original's minus exactly the removed ones, its parse tree must keep the original's shape (so removing a `;` can't join two statements or turn one into another's body), and linting it must find the original's other violations and nothing new. A file that fails a check is left as it was and listed as `path not fixed: <why>` on stderr. A file that doesn't parse is never fixed. Writes go through the same path as formatting: symlinks, permissions, owners, hard links and `exclude` are handled the same way. Running it again changes nothing.

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

`continuation_indent = 8` matches IntelliJ's default and most Grails projects written in it. On OpenBoxes it cuts the first-run diff from 723 files to 453, and what remains is mostly trailing whitespace and runs of blank lines.

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

The Gradle plugin (`gradle-plugin/`) adds three tasks: `groovyfmtCheck` and `groovyfmtLint`, which `check` runs, and `groovyfmtFormat`, which formats in place. It downloads the release binary for the machine (Linux or macOS, x86_64 or arm64) into `~/.gradle/groovyfmt/<version>/<target>/`, checks it against its `.sha256` before unpacking it, and reuses it after that. It needs Gradle 7.6 or later on Java 11 or later, and works with the configuration cache.

It is not on the Gradle Plugin Portal yet. Until it is, build it from a checkout of this repository with `includeBuild`:

```groovy
// settings.gradle
pluginManagement {
    includeBuild '../groovyfmt/gradle-plugin'  // path to your checkout
}
```

```groovy
// build.gradle
plugins {
    id 'base'  // or any plugin that adds check, such as java
    id 'io.github.costajohnt.groovyfmt'
}

groovyfmt {
    // version = '0.8.0'                          // default: the plugin's own version
    // binary = file('/usr/local/bin/groovyfmt')  // run this binary and download nothing
    // args = ['--strict']                        // added to every run
    // paths = ['src', 'build.gradle']            // default: the project directory
}
```

Once it is published, drop `includeBuild` and write `id 'io.github.costajohnt.groovyfmt' version '<version>'`.

A task fails with groovyfmt's own output when files need formatting or lint finds something, and with a separate message when groovyfmt itself fails (exit code 2: bad arguments, a bad config, an unreadable file). The tasks are never up to date: groovyfmt finds its own files and configs, so Gradle can't tell when they changed.

Apply the plugin to the root project only. groovyfmt searches the whole directory tree, subprojects included (it skips `build` directories), so one run covers a multi-project build; applying it to subprojects too would check their files twice.

While the repository is private, the download needs a token that can read it: the environment variable `GROOVYFMT_GITHUB_TOKEN` or `GITHUB_TOKEN`, or the Gradle property `groovyfmt.githubToken` in a `gradle.properties` file. The token is sent only to `api.github.com`, in a header. Don't pass it as `-P` on the command line: Gradle 7.6 keeps every command-line `-P` value, in plain text, in its configuration cache. With `--offline` the plugin uses the cached binary and fails if there is none. There is no Windows release binary; on Windows, set `binary`.

Without the plugin, with `groovyfmt` on `PATH`:

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

groovyfmt only touches files it can parse. On a corpus of 23,766 files from grails-core, groovy, gradle, spock, CodeNarc, nextflow and Jenkins pipeline repositories, 99.6% parse cleanly. On [OpenBoxes](https://github.com/openboxes/openboxes), a production Grails application that isn't in the corpus, 99.5% of its 1,327 files do.

The grammar is [dekobon/tree-sitter-groovy](https://github.com/dekobon/tree-sitter-groovy) as extended on the fork branch [`costajohnt/tree-sitter-groovy@groovyfmt-base`](https://github.com/costajohnt/tree-sitter-groovy/tree/groovyfmt-base), which adds upstream PR #49 plus our fixes to the 0.3.0 release. Its generated parser is vendored in [`grammar/`](grammar/README.md), which records the fork commit it was copied from:

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
| + Groovy 5 and 6 forms (`for await`, `case Integer i when`, indexed `for (i, x in xs)`, `??`); `\\/` in slashy strings; a comparison right after a name (`i<n ? a : b`); quoted command names (`'x:title' lang: 'en'`) and calls chained onto calls; `new`, `switch`, `in` and `static` as names; nested array initializers and `{a, b}` annotation arrays; spaced and annotated type arguments and smaller forms | 99.6% |

Of the 85 corpus files still skipped, 80 aren't Groovy source: 70 Grails, Gradle and CodeNarc templates with `@artifact.package@`, `<% %>` or `${className}` placeholders, 9 Spock AST snapshots and one deliberately broken groovydoc fixture. The other 5 use forms the grammar doesn't parse yet: Groovy 5 list patterns (`case [var only] ->`) and map destructuring (`def (from: a, to: b) = r`), a guard whose last operand is a bare name before `->`, a multiple assignment and comma-separated updates in a `for` header, and a spread first command argument (`bridgedClasses *list, 'x'`).

## Development

```sh
scripts/ci-local.sh [corpus-dir]         # everything CI checks on Linux; run it before merging
cargo test                               # unit and end-to-end CLI tests
cargo clippy --all-targets -- -D warnings
groovyfmt --stats <dir>                  # corpus report, including an idempotence check
groovyfmt --errors <dir>                 # first parse error per failing file, for grammar work
groovyfmt --tree <file>                  # dump the parse tree
```

The grammar in `grammar/` is a copy; don't edit it here. To change it, fix `grammar.js` or `src/scanner.c` in a [tree-sitter-groovy](https://github.com/costajohnt/tree-sitter-groovy) checkout, run `tree-sitter generate` and its tests there and commit, then run `scripts/update-grammar.sh <checkout> [rev]` here (the checkout must be clean; `rev` defaults to its HEAD) and compare `--stats` and `--lint` on the corpus before and after.

Releases: push a `v*` tag on `main` matching the Cargo.toml version and with a CHANGELOG heading; the release workflow publishes only after those checks, fmt, clippy, the tests on stable and Rust 1.90, and a build and smoke test of all four binaries pass.

## License

MIT
