# Command line

```text
groovyfmt [OPTIONS] [PATH]...
```

`groovyfmt --help` prints the same reference in short form.

## Modes

| Command | Does |
|---|---|
| `groovyfmt` | format files in place |
| `groovyfmt --check` | list files that would be reformatted, exit 1 if any |
| `groovyfmt --check --diff` | print a unified diff of what formatting would change, exit 1 if any |
| `groovyfmt --lint` | report lint violations, exit 1 if any |
| `groovyfmt --lint --fix` | fix what can be fixed safely (unused and duplicate imports, imports from the file's own package, unnecessary semicolons), then report the rest |
| `groovyfmt --syntax` | only parse, and report each place groovyfmt could not parse, exit 1 if any |
| `groovyfmt --stdin` | format stdin to stdout, for editors |
| `groovyfmt --lsp` | serve the Language Server Protocol on stdin and stdout ([editors](editors.md)) |
| `groovyfmt --rules` | list lint rules and whether each is on |
| `groovyfmt --explain <Rule>` | explain a lint rule: why it matters, an example, whether `--fix` fixes it, how to turn it off |
| `groovyfmt --suggest-style` | suggest the indent settings that fit the code, writing nothing ([configuration](configuration.md#finding-the-settings-that-fit)) |

## Options

| Option | Effect |
|---|---|
| `--config <file>` | use this config for every file ([configuration](configuration.md)) |
| `--indent <n>` | override the indent of whichever config applies |
| `--stdin-filepath <path>` | with `--stdin`, find the config as if formatting this path ([editors](editors.md#formatting-through-stdin)) |
| `--output-format text\|github\|json\|sarif` | see [output formats](output-formats.md) |
| `--strict` | list skipped files and exit 1 if there are any, with `--check`, `--lint` or in place |
| `--force` | read every file named on the command line as Groovy, whatever its name |
| `--diff` | with `--check` or `--lint --fix`, print a diff and write nothing ([below](#diffs)) |
| `--lines <start>-<end>` | change only these lines ([below](#formatting-some-lines)) |
| `--changed-since <rev>` | only files that differ from a git revision ([below](#only-changed-files)) |
| `--baseline <file>`, `--write-baseline[=file]` | lint against a baseline ([lint](lint.md#baseline)) |
| `-v` | also list skipped files, where each failed to parse, and timing |
| `--` | treat every later argument as a path |

## Exit status

| Status | Meaning |
|---|---|
| 0 | clean |
| 1 | files need formatting, lint found something, a file doesn't parse (`--syntax`), or, with `--strict`, files were skipped |
| 2 | an error: bad arguments, a bad config, an unreadable file, or `--stdin` input that can't be formatted |

## Which files

With no paths, groovyfmt works on the current directory.

Directories are searched for `*.groovy`, `*.gradle`, `*.gvy`, `*.gy`, `*.gsh` and `Jenkinsfile`. The search skips:

- hidden directories
- `build`, `out`, `target` and `node_modules`
- anything matching the config's [`exclude`](configuration.md#exclude) patterns

A file named on the command line is processed even inside a skipped directory, unless it or a directory above it matches `exclude`.

A file named on the command line must still be Groovy by its name. That means one of:

- a name a directory search would find
- a pipeline named `Jenkinsfile.*` or `*.jenkinsfile`
- a script whose first line is a `#!` that runs groovy

Any other named file (a Makefile or YAML file passed by `groovyfmt *`) is skipped and listed under `-v` or `--strict` as `not a Groovy file`. `--force` reads every named file as Groovy whatever its name.

These files are skipped and listed under `-v` or `--strict`:

- files that aren't valid UTF-8
- files that aren't regular files
- files larger than 10 MB
- files that would need more than 200 columns of indentation (50 levels at the default indent)
- files that don't parse ([below](#parse-errors))

### Symlinks

- Symlinks inside searched directories are not followed.
- A symlink named on the command line is followed. A linked file is formatted and the link stays a link. A linked directory is searched.
- A named link (to a file or a directory) whose target is outside the working directory is read by `--check` and `--lint`, but nothing is rewritten through it. Formatting it in place is an error.
- This applies only when the argument itself is a link. `../other-project` or an absolute path is formatted as usual.

## Parse errors

A file groovyfmt can't parse is never formatted or linted. `--syntax` lists where parsing failed, one line per place, up to 10 per file and then a count of the rest. `-v` and `--strict` show the same lines under each skipped file, and `--stdin` prints them after its error.

```console
$ groovyfmt --syntax
Jenkinsfile:4:14: could not parse: `{` is never closed in a closure
src/Build.groovy:12:21: could not parse: expected `)` to close the `(` on line 12
could not parse 2 of 31 files
```

Columns count characters. The message names the bracket left open when there is one. Otherwise it quotes the text the grammar stopped at and says where it was (in an argument list, at the top level).

The message only means groovyfmt could not parse the code. The grammar is not the Groovy compiler, and about 0.5% of valid files don't parse ([coverage](coverage.md)).

Exit status is 0 when every file parses, 1 when one doesn't, and 2 on errors. `--output-format github` turns each place into an annotation.

## Changing less

### Diffs

`--check --diff` prints a unified diff of what formatting would change instead of the file names, and writes nothing. `--lint --fix --diff` does the same for the fixes. Exit statuses don't change.

The diff names files `a/<path>` and `b/<path>` as the text output names them, so from the same directory `git apply` or `patch -p1` applies it.

A diff has to print file content as it is. So a `# groovyfmt: <path>: diff not shown` line takes the place of the diff (both tools skip it) for a file when:

- its name holds a control character, `"` or `\`
- its diff would print a control character other than a tab, `##[`, or a line starting with `::`

That keeps a file name or content from forging a patch header or acting on a terminal or a CI log. The name is checked before it goes into the `---` and `+++` lines, and the patch is checked as a whole. Files with CR LF line endings diff as they are, since that CR only ends a line. A CR anywhere else withholds the diff.

### Formatting some lines

`--lines <start>-<end>` changes only those lines, for one file or `--stdin`, as editors do for a selection. Ranges are 1-based and inclusive, and the option can be repeated. Every other line comes out byte for byte as it was.

Indentation comes from the whole parse tree, so the whole file is formatted and the original text put back outside the ranges. The result goes through the same token check as formatting.

No line is added or removed. A blank line formatting would drop is emptied instead, so the lines after a range keep their numbers, and running it again with the same ranges changes nothing. The exception is [line wrapping](formatting.md#line-wrapping), whose pieces replace a wrapped line inside a range.

### Only changed files

`--changed-since <rev>` limits a run (formatting, `--check`, `--lint`, `--syntax`) to files under the working directory that differ from git revision `rev` in the working tree or index, plus untracked files git doesn't ignore. Those files are formatted whole.

It runs `git diff --name-only -z --relative <rev> --` and `git ls-files -z --others --exclude-standard`. `rev` can't start with `-`. A missing git, or a directory outside a repository, is an error (exit 2).
