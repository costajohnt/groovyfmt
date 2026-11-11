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
| [Release binaries](#release-binaries) | available; [Windows](#windows) since 0.9.1, tested on Windows Server 2022 and under Wine |
| [Build from source](#build-from-source) | available |
| [pre-commit](#pre-commit) | available since 0.9.1 |
| [GitHub Action](#github-actions) | available since 0.9.1; while the repository is private, only with a token that can read it |
| [Gradle plugin](#gradle) | not published yet; works today from a checkout with `includeBuild` |
| [npm](#npm) (`npx groovyfmt`) | not published yet; comes once the repository is public |
| [Homebrew](#homebrew) | not published yet; comes once the repository is public |
| [Docker image](#docker) | not published yet; comes once the repository is public |

### Release binaries

Prebuilt binaries for Linux (x86_64 and arm64, both static musl builds that need no glibc), macOS (Apple Silicon, Intel) and [Windows](#windows) (x86_64, a `.zip`) are attached to each [release](https://github.com/costajohnt/groovyfmt/releases), each with a `.sha256` file:

```sh
gh release download --repo costajohnt/groovyfmt --pattern '*aarch64-apple-darwin.tar.gz*'
shasum -a 256 -c groovyfmt-*.tar.gz.sha256
tar -xzf groovyfmt-*.tar.gz && mv groovyfmt-*/groovyfmt /usr/local/bin/
```

### Windows

Support level: **tested on Windows Server 2022 (x86_64, NTFS) and under Wine; not yet on a desktop Windows release or on ARM.** `groovyfmt-<version>-x86_64-pc-windows-gnu.zip` holds `groovyfmt.exe`, one file that needs only Windows' own DLLs: it runs on a Windows machine that has never had a compiler or runtime installed. It is cross-compiled on Linux with mingw-w64. The whole test suite passes on Windows Server 2022 and under Wine, and formatting a 3,000-file sample of the corpus on Windows Server, and the whole 23,766-file corpus under Wine, gives files byte-identical to the Linux build's. In PowerShell:

```powershell
gh release download --repo costajohnt/groovyfmt --pattern '*windows*'
(Get-FileHash groovyfmt-*.zip -Algorithm SHA256).Hash.ToLower()   # compare with the .sha256 file
Expand-Archive groovyfmt-*.zip .
```

Then put the folder holding `groovyfmt.exe` on `PATH`. `tar -xf` (the `tar.exe` Windows ships) unpacks the zip too. The [GitHub Action](#github-actions), the [npm](#npm) package (`@groovyfmt/win32-x64`) and the [Gradle plugin](#gradle) install it on Windows x64 too. There is no Windows on ARM build.

What to expect (each checked on Windows Server 2022 unless it says otherwise):

- Output is the same as on Linux and macOS: paths print with `/` in every format (text, `github`, JSON, SARIF, `--diff` headers), and groovyfmt never turns `\n` into `\r\n`, so CRLF files keep their line endings and `--lsp` framing stays exact over pipes. Paths can be typed with `\` or `/`, with or without a drive letter. `exclude` patterns use `/` and are case-sensitive, as everywhere, so a config written on Linux works unchanged.
- Formatting in place writes a temporary file beside the original and renames it over it, as elsewhere, so a crash never leaves half a file. A read-only file is not formatted (exit 2). A file another program has open is formatted if that program opened it with delete sharing (`FILE_SHARE_DELETE`); otherwise groovyfmt names the file, exits 2 and leaves it as it was. Before the rename, groovyfmt checks that the path still holds the file it read by size, modification time and contents (Windows has no inode number std can read).
- The rewritten file gets the permissions (ACL) its folder gives new files, so a file with explicit permissions of its own loses them. There is no owner to keep.
- Hard links can't be seen, so formatting a hard-linked file replaces it and its other names keep the old text. On Linux and macOS they stay linked.
- Symlinks and junctions get the same rules as on Linux and macOS: not followed inside a walk, followed when named, and not written through when named and pointing outside the working directory (`groovyfmt: vendor: symlink target is outside the working directory, not formatted`, exit 2).
- `CON`, `NUL`, `COM1` and the other device names are never created or read: naming one is `no such file or directory` (exit 2). Windows drops a trailing dot or space from a name, so `A.groovy.` is `A.groovy`. Paths longer than 260 characters work, without the `LongPathsEnabled` setting.
- Neither PowerShell nor `cmd` expands `*.groovy` for a program on Windows, and groovyfmt doesn't either: name a directory (`groovyfmt --check .`), which is searched as on Linux, or let PowerShell expand the names (`groovyfmt --check (Get-ChildItem *.groovy).Name`).
- `--stdin` reads and writes bytes, but Windows PowerShell 5.1 re-encodes what it pipes to a program (as ASCII, by default, so `é` becomes `?`) and adds line endings, and its `>` writes UTF-16. Use `cmd`'s redirection, which passes bytes through: `cmd /c "groovyfmt --stdin < A.groovy > out.groovy"`. Editors and the `--lsp` server don't go through a shell and aren't affected. File names print as UTF-8; to read non-ASCII ones in a PowerShell 5.1 console, first run `[Console]::OutputEncoding = [Text.Encoding]::UTF8`. In PowerShell 5.1 a quoted argument can't end in `\` (`'my dir\'` reaches groovyfmt as `my dir"`): write `'my dir'` or `'my dir/'`.
- `--changed-since` runs `git` from `PATH` (Git for Windows); without it, it exits 2 with `running git: program not found`. Not yet tried with Git for Windows installed.

Not checked yet: desktop Windows 10 and 11, Windows on ARM, antivirus or indexer locks on a file being replaced, PowerShell 7, and Git for Windows.

### Build from source

Rust 1.90+ and a C compiler:

```sh
cargo install --locked --git https://github.com/costajohnt/groovyfmt
```

The Groovy grammar is vendored in the repository (`grammar/`), so this fetches only groovyfmt and its crates.io dependencies.

### npm

Not published yet. Once it is, `npx groovyfmt` runs it without installing, and `npm install --save-dev groovyfmt` adds it to a project. The package downloads only the binary for your platform (Linux or macOS, x64 or arm64, or Windows x64).

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
| `groovyfmt --lint --fix` | fix what can be fixed safely (unused and duplicate imports, imports from the file's own package, unnecessary semicolons), then report the rest |
| `groovyfmt --syntax` | only parse, and report each place groovyfmt could not parse, exit 1 if any |
| `groovyfmt --stdin` | format stdin to stdout, for editors |
| `groovyfmt --rules` | list lint rules and whether each is on |
| `groovyfmt --explain <Rule>` | explain a lint rule: why it matters, an example, whether `--fix` fixes it, how to turn it off |
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
| `summary` | counts of files by status (`errors` counts `error`) and of diagnostics; with a baseline in use, also `baselined` (violations it held back) and `stale_baseline_entries` |

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

Whitespace only (with `max_line_length`, also line breaks: see Line wrapping):

- every line is re-indented from the parse tree (blocks, closures, argument lists, list and map literals, continued expressions, Spock `when:` / `then:` blocks)
- trailing whitespace is removed, runs of blank lines collapse to one, and the file ends with one newline
- lines inside multi-line strings and comments are left exactly as they are

Formatting is safe by construction. The output is parsed again and its token stream compared with the input's; if anything other than whitespace would change, or the input doesn't parse cleanly, the file is left untouched (`-v` lists these). Running it twice changes nothing the second time.

### Spacing within lines

With `normalize_spacing = true` in the config, groovyfmt also fixes the spaces between tokens on each line. It never adds or removes a line break and never touches strings, GString interpolations or comments. The style:

- one space around binary operators, assignment (`=`, `+=`), `?` and `:` in a ternary, `?:`, `->`, `**`, `as`, `in` and `instanceof`
- one space after a comma and none before it; none before a `;`, one after it inside a `for (…)`
- no space inside `(…)` or `[…]`, and none inside `<…>` of a generic type (`Map<String, Integer>`)
- no space between a call, constructor, annotation or method declaration and its `(` (`foo(x)`, `new Foo()`, `@Ann(x)`, `def f(a)`), one after `if`, `for`, `while`, `switch`, `catch` and `synchronized`
- one space before a `{`, inside a one-line `{ … }` and around `else`, `catch` and `finally` after a `}`
- in map entries, named arguments, labels and `case x:`, no space before the `:` and one after it in map entries and named arguments
- no space after a prefix operator (`-x`, `!x`, `*args`) or before a postfix one (`i++`), around `::`, or before the `[` of an array type (`String[]`), and one after a cast (`(String) x`)
- any other run of spaces becomes one

Some spacing is left as written because changing it could change what the code means or how it reads: member access (`.`, `?.`, `*.`, `.&`), ranges (`1..n`), a subscript after a space (`a [1]`), a call with a space before `(` whose result is used further (`foo (x).bar()`, `foo (x) + 1`, which Groovy may read as a command call), a binary operator with a space before it and none after (`a -b`, `println -1`), `<` and `>` comparisons chained like a generic type (`x as Kind<Path>[]`), a closure parameter's default (`{ int b=2 -> }`), and closures that look like template placeholders (`{{name}}`, `${x}` outside a string, a closure standing alone as a statement).

Alignment is kept. A run of two or more spaces, or a tab, stays when the token after it lines up with a token on a nearby line: the line above or below, or a line indented the same way within 20 lines (an `=` column, map keys and values across nested values, Gradle dependency columns, trailing comments). A single space before a map key's `:` stays when it lines up colons (`9 :` over `10:`). When something earlier on the line gets shorter or longer, a run before a trailing comment absorbs the difference so the comment doesn't move, and on a line laid out in columns (`[ 4,  4,  3]` over `[10,  9,  8]`) nothing before its last aligned column changes. Spock data tables are never touched: every statement from a `where:` label to the end of its block keeps its spacing exactly. A gap next to a comment is kept too.

Besides the token check, spacing output must parse to the same tree shape as the input; if it doesn't, the file is formatted without spacing changes instead.

Limits, so a hostile file can't make it slow: a line longer than 1,000 bytes keeps its spacing (it is still re-indented), and alignment is looked up only in the first 2,000 characters of a line. Lines whose alignment depends on each other are settled in repeated passes over the file, each linear in its size; two or three are typical, and a file that hasn't settled after 64 is skipped (`-v` lists it as `spacing did not settle`) rather than written half-respaced.

### Line wrapping

With `max_line_length = <n>` in the config (40 to 400; there is no default, and without the key nothing is wrapped), groovyfmt breaks lines longer than `n` columns where that is safe. A tab counts as `indent` columns, and width is counted in characters, not bytes. Wrapping only ever adds line breaks: it never joins lines, never breaks inside a token, and never touches a line inside a string or comment.

In Groovy a line break can end a statement, so a break goes only where Groovy's own grammar (the Parrot parser's `GroovyLexer.g4` and `GroovyParser.g4`) reads the code the same with or without it:

- inside `(…)` and `[…]`, where Groovy's lexer drops newlines before the parser sees them: an argument list, list literal or map literal with two or more items breaks one item per line, after the opening bracket and each comma, with the closing bracket on a line of its own. A trailing closure stays on the line of the `)` (`) {`)
- before each `.`, `?.` or `*.` of a chain of three or more calls, except the first call's (`Foo.builder()`, then `.name(x)` and `.build()` on lines of their own), which the grammar allows anywhere (`pathElement : NL* DOT …`). In a chain already written over several lines, the dots on a line that is still too long break the same way
- after `||`, `&&`, `==`, `!=`, `+` and `-` at the lowest-precedence level of a binary expression, after a ternary's `?` and `:`, and after `?:`. The operator always ends the line: the grammar takes a newline after `+` and `-` but not before them, and `a` then `-b` on the next line would be two statements
- after `=` in an assignment or declaration, when nothing else on the line can break

For each line over the limit, the outermost construct that lies wholly on that line breaks one of these ways; the file is re-indented (and respaced, with `normalize_spacing`), the lines are measured again, and pieces still too long break further. Continuation lines get `continuation_indent`. An operator or `=` breaks only when every resulting piece fits, so `a.b().c().d() == x` breaks the chain and `x == [a, b, c]` the list instead, and a single binary operator doesn't leave a piece shorter than 8 columns (`f(x) ==` then `1`).

Never broken, because Groovy or the grammar might read the result differently, or because the result would read worse than the long line:

- anything in a parenthesis-free command call (`implementation group: 'a', name: 'b'`, `input 'x', options: [a, b]`), whose arguments tree-sitter-groovy finds by looking at line ends, and a statement that starts on the line where the one before it ends without a `;`, which is how the grammar reads some command calls (`println a + b` as `println` and `a + b`)
- anything inside a closure or block written on one line (`list.each { foo(a, b) }`): its statements end at line breaks
- between `return`, `throw`, `assert`, `new`, `as`, `in`, `instanceof` or `!` and what follows, between a name and its `(`, between `)` and a trailing closure, and anywhere the line before the break could end a statement by itself. Outside `(…)` and `[…]` a wrapped line never starts with `{`, `++`, `--`, `+`, `-`, `*` or `/` (a `*.` of a chain aside), and starts with `(`, `[` or a string only right after a line ending in a binary operator, `?`, `:` or `=`, which the grammar says must continue. Inside brackets Groovy's lexer drops the line break, so an argument such as `-1` or `'text'` may start a line
- annotations, `import` and `package` lines, `case` labels, Spock `where:` blocks, GString interpolations, and argument lists or literals holding a comment
- `<`, `>`, `as`, `instanceof`, `in`, ranges, shifts and the other operators: only the ones above are break points. Nothing breaks right after a `>` either: tree-sitter-groovy reads `f(a, List<T>` followed by a line break as comparisons
- a part of a larger construct on the line that can't break itself: an operator expression inside another operator's operand (`(a != null && b) ? x : y`), in parentheses (`(a ?: b).c()`) or in a subscript (`a[i + 1]`), an operator expression or chain that is a call's only argument (`f(a + b)`, also through a cast) or a lambda's body, anything in a one-line ternary's condition or first branch (`c ? f(a, b) : d`), the only argument written `key: value`, the condition of an `assert` with a message or of an `if` or `while` whose body follows on the same line (braces or not), and whatever is inside something that may not break
- an argument list or literal followed on its line by more text than it holds (`f(_, _) >> { long closure parameters ->`): breaking it gains little
- a line holding two statements (`a(x, y); b(z)`) or an `if`, `else`, `while` or `for` body without braces (`if (c) x = f(a, b)`), and a line of a list written over several lines that holds two or more of its items (laid out by hand)
- a line whose overflow is only a trailing comment, or that holds a string too long to fit on a continuation line anyway

A line that is still too long after every safe break stays as it is. Each pass parses the result again and must find the same tree shape, and the final text must have the same tokens and tree shape as the input, or the file is formatted without wrapping instead. Formatting the output again changes nothing, for any combination of the other settings. Wrapping goes at most 16 constructs deep on one line, and isn't tried on a file nested more than 1,000 levels deep; such files are formatted without wrapping. Lines between `// groovyfmt: off` and `// groovyfmt: on` are never wrapped. With `--lines`, only lines in the ranges are wrapped, and a wrapped line's pieces take its place, so the lines after it move down. `-v` lists each file formatted without wrapping and why (`not wrapped <file>: <reason>`). `--suggest-style` ignores `max_line_length`: it measures indent styles only.

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
| UnusedSuppression | off | a `groovyfmt-ignore` comment that suppresses nothing (or names no rule), or a CodeNarc comment or `@SuppressWarnings` naming a groovyfmt rule it doesn't suppress (see [Suppressing reports](#suppressing-reports)) |
| ComparisonWithSelf | off | a variable compared with itself (Spock equality checks do this on purpose); not inside a GORM where query (`Book.where { title == title }`, `@Where`), where the left side is the domain property |

A block holding only a comment doesn't count as empty.

#### Imports

| Rule | Default | Flags |
|---|---|---|
| UnnecessaryGroovyImport | on | an import from `java.lang`, `java.util`, `java.io`, `java.net`, `groovy.lang` or `groovy.util`, or of `java.math.BigDecimal` or `BigInteger`, which Groovy imports by default; not an aliased or static import, not in a file with another star import or a class of that name (either can be what the import is there to beat), and not in `*.gradle` files, where Gradle's own default imports (`org.gradle.api.tasks.Optional`) can need it |
| ImportFromSamePackage | on | an import of a class (or `.*`) from the file's own package |

#### Likely bugs

Rules marked off found, on real code, almost only things written that way on purpose (mostly in tests); turn them on where that doesn't apply.

| Rule | Default | Flags |
|---|---|---|
| DeadCode | on | a statement after `return`, `throw`, `break` or `continue` in the same block, on a later line; not a method or class declaration (a script's come after its `return`), a labeled statement, or a `break` right after a `return` or `throw` (redundant, not wrong) |
| EqualsAndHashCode | on | a class defining `equals(Object)` without `hashCode()`; not one with a superclass (which may define it) or an annotation that generates it (`@EqualsAndHashCode`, `@Canonical`, `@Immutable`) |
| EqualsOverloaded | on | an `equals` method with one parameter typed other than `Object`, which overloads `equals` instead of overriding it |
| ObjectOverrideMisspelledMethodName | on | `equal(Object)`, `hashcode()`, `toSTring()` and other case misspellings of `equals`, `hashCode` and `toString` |
| ReturnFromFinallyBlock | on | a `return` in a `finally` block (not in a closure inside it) |
| EmptySwitchStatement | on | a `switch` with no cases |
| EmptyStaticInitializer | on | an empty `static { }` |
| EmptyInstanceInitializer | on | an empty `{ }` instance initializer |
| BrokenNullCheck | on | `x != null \|\| x.a` and `x == null && x.a`, which dereference `x` exactly when it is null |
| RemoveAllOnSelf | on | `c.removeAll(c)` instead of `c.clear()` |
| MissingNewInThrowStatement | on | `throw SomeException(...)` or `throw SomeException` without `new` (a capitalized name ending in `Exception`, `Failure` or `Fault`) |
| ExceptionNotThrown | on | a `catch` block ending in `new SomeException(...)` that is neither thrown nor the method's or closure's return value |
| ThrowExceptionFromFinallyBlock | off | a `throw` in a `finally` block, unless a `catch` inside the `finally` may handle it (rethrowing a saved exception after cleanup is a common idiom) |
| AssertWithinFinallyBlock | off | an `assert` in a `finally` block (tests check state after cleanup this way) |
| ToStringReturnsNull | off | a `toString()` that returns `null`, ends with `null`, or is empty (found only test fixtures doing it on purpose) |
| DuplicateSetValue | off | `[...] as Set` (or any type ending in `Set`) with the same constant twice (tests do it to check that a set drops them) |
| ConstantTernaryExpression | off | a ternary or elvis whose condition is a literal, `Boolean.TRUE` / `FALSE`, or a list or map literal |
| ConstantAssertExpression | off | an `assert` whose condition is a constant (`assert false` marks unreachable code in tests) |
| ComparisonOfTwoConstants | off | `==`, `!=`, `<`, `<=`, `>`, `>=`, `<=>`, `equals` or `compareTo` between two constants, or lists and maps of them (tests and `expect: 1 == 1` placeholders) |
| BrokenOddnessCheck | off | `x % 2 == 1`, false for negative odd numbers (nearly every real case is a count that is never negative) |

#### Unused code

A name counts as used wherever it appears outside comments: as a name, or as a word in any string (`"$x"`, `getProperty('x')`, `@MethodSource('x')`). That can miss an unused one whose name is used elsewhere for something else, but never reports a used one. A member reached through a computed name, `this."groupBy_${kind}"` or `"parse_$kind"()`, counts as used when its name starts with the literal part, and `getProperty(name)`, `invokeMethod(name, args)` or `this[name]` with a computed name count as a use of every private member, as does any mention of `metaClass`. A declaration the grammar misreads, `final ClassNode name` without a value (read as a variable named `ClassNode`), is left alone.

| Rule | Default | Flags |
|---|---|---|
| UnusedPrivateField | on | a private field whose name appears nowhere else in the file; not `serialVersionUID`, an annotated field (`@Delegate`, `@Autowired`; `@SuppressWarnings` and `@Deprecated` don't count), or a field of a class whose annotation reads fields (`includeFields`, `@Entity`, `@Builder`) |
| UnusedPrivateMethod | on | a private method whose name appears nowhere else in the file (for `getX`, `isX` and `setX`, nor the property `x`); not an annotated method, or one called by name: `methodMissing`, `readObject` and the like, Spock's `setup` / `cleanup` / `setupSpec` / `cleanupSpec`, GPars' `afterStart` / `afterStop` / `onException` / `onInterrupt` / `onTimeout` |
| UnusedVariable | on | a local variable whose name doesn't appear after its declaration in its block, closure or switch; not a script's top-level variable, `_`, an annotated one (`@Field`), or a part of `def (a, b) = ...`. Assigning it counts as a use |

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

`groovyfmt --explain <Rule>` prints a rule's description, why it matters, flagged code and a fixed version, whether `--lint --fix` fixes it, and the `.groovyfmt.toml` lines to turn it off (or on).

### Fixing

`groovyfmt --lint --fix` removes, in place, what four rules flag: an `UnusedImport`, `DuplicateImport` or `ImportFromSamePackage` line (with its line break, and only when nothing but whitespace or a `;` shares the line) and an `UnnecessarySemicolon` `;`. An import from the file's own package is removed only when the file declares no class of the same name; an `UnnecessaryGroovyImport` is never removed, since it can be what beats a same-named class in the package, which one file can't show. It then reports the violations that remain and exits 1 if there are any. Plain `--lint` marks these violations with ` (fixable)` (`"fixable": true` in JSON). A rule turned off in the config is neither fixed nor reported.

```console
$ groovyfmt --lint --fix
src/A.groovy:4:1: UnusedImport: The [b.C] import is never referenced
fixed 4 violations in 1 files (DuplicateImport 1, UnnecessarySemicolon 2, UnusedImport 1)
1 violations in 3 files (UnusedImport 1)
```

As with formatting, each fix is checked before the file is written: the fixed file must parse cleanly, its tokens must be the original's minus exactly the removed ones, its parse tree must keep the original's shape (so removing a `;` can't join two statements or turn one into another's body), and linting it must find the original's other violations and nothing new. A file that fails a check is left as it was and listed as `path not fixed: <why>` on stderr. A file that doesn't parse is never fixed. Writes go through the same path as formatting: symlinks, permissions, owners, hard links and `exclude` are handled the same way. Running it again changes nothing.

### Suppressing reports

A comment can turn rules off for part of a file. Without a rule list it covers every rule; rule names are separated by commas or spaces, and `//` and `/* */` comments both work.

| Comment | Covers |
|---|---|
| `// groovyfmt-ignore [Rule, ...]` | its own line when it follows code there; the next line when it stands alone |
| `// groovyfmt-ignore-start [Rule, ...]` ... `// groovyfmt-ignore-end` | every line from the start comment to the end comment (or to the end of the file) |

CodeNarc's own forms work unchanged, so a project moving from CodeNarc keeps its suppressions:

| Form | Covers |
|---|---|
| `// codenarc-disable [Rule, ...]` ... `// codenarc-enable [Rule, ...]` | from the disable comment to an enable comment for the same rules (or the end of the file); a bare `codenarc-enable` ends them all |
| `// codenarc-disable-line [Rule, ...]` | its own line |
| `@SuppressWarnings('Rule')`, `@SuppressWarnings(['A', 'B'])` | the class, method, constructor, field or variable it annotates; on an import, the whole file. `'all'` and `'CodeNarc'` mean every rule |

A report is suppressed by the line it starts on. Suppressed reports are left out everywhere: the text, `github`, JSON and SARIF output, the exit status, `--lint --fix` (which doesn't fix them) and the language server. The off-by-default rule `UnusedSuppression` reports a groovyfmt comment that suppresses nothing, and a CodeNarc comment or `@SuppressWarnings` naming a groovyfmt rule that it doesn't suppress (other names, such as `unchecked` or CodeNarc-only rules, are left to their tools). It judges every rule, including ones turned off in the config.

Formatting has its own pair: the lines from `// groovyfmt: off` to `// groovyfmt: on` (both comments included, or to the end of the file) come out byte for byte as they were. The rest of the file is formatted as with `--lines`, so in a file with an `off` comment no line is added or removed.

### Baseline

A baseline records the violations a repository has today, so the linter can be turned on for new code without fixing the old first:

```console
$ groovyfmt --lint --write-baseline
wrote 1204 violations in 312 files to .groovyfmt-baseline.json
```

With `baseline = ".groovyfmt-baseline.json"` in `.groovyfmt.toml`, or `--baseline FILE` on the command line, `--lint` then reports only violations that aren't in it, and exits 0 when those are all there is. The summary counts the ones the baseline held back and its stale entries (recorded violations that are gone):

```console
$ groovyfmt --lint
src/Build.groovy:12:5: DeadCode: This code can never run: ...
1 violations in 312 files (DeadCode 1), 1180 in the baseline, 24 stale baseline entries (--write-baseline drops them)
```

`--write-baseline` writes to `--write-baseline=FILE` (with `=`: a separate word would be a path to lint), else the `--baseline` file, else the config's `baseline`, else `.groovyfmt-baseline.json` in the working directory. Run it again to drop stale entries; entries for files the run didn't lint are kept. With `--strict`, stale entries fail the run (exit 1), so the baseline can only shrink: fix a violation, then rewrite the baseline. `--lint --fix` leaves baselined violations alone (any violation of the same rule on a line the baseline recorded); pass `--baseline ''` to ignore the baseline and fix them too. JSON and SARIF output leave baselined violations out, as they do suppressed ones; SARIF doesn't list them as suppressed results.

Each entry is the file's path relative to the baseline file's directory (with `/`), the rule, a fingerprint of the text of the line the violation starts on (whitespace runs collapsed, so re-indenting doesn't matter), and its index among same-fingerprint violations of that rule in that file. Line numbers aren't recorded, so edits above a violation don't disturb it; editing the line itself (beyond its spacing or its final `;`), or renaming or moving the file, makes its violations new. The file is sorted JSON, the same on every machine, so it diffs cleanly in review:

```json
{"version":1,"entries":[
{"path":"src/Build.groovy","rule":"UnusedImport","fingerprint":"5f1c0d4a7b2e9c13","index":0}
]}
```

A baseline that isn't valid JSON, has another `version`, isn't a regular file or is over 10 MB is an error (exit 2), never silently ignored; the message names the file and the problem, never its content. Paths inside it are only compared as text, never opened.

A config can come from an untrusted pull request, so its `baseline` must be a relative path inside the config's own directory, without `..` (anything else is a config error), and groovyfmt neither reads nor writes it through a symbolic link that leads out of that directory. A file named on the command line (`--baseline FILE`, `--write-baseline=FILE`) is your own choice and can be anywhere. The language server honors the config's baseline too: what it holds isn't published, and its fixes leave it alone.

Adopting groovyfmt lint in an existing repository:

1. `groovyfmt --lint --rules` and turn off, in `.groovyfmt.toml`, any rule the project doesn't want.
2. `groovyfmt --lint --fix` to remove what can be removed safely.
3. `groovyfmt --lint --write-baseline` and commit `.groovyfmt-baseline.json`.
4. Add `baseline = ".groovyfmt-baseline.json"` to `.groovyfmt.toml`.
5. Run `groovyfmt --lint --strict` in CI, and rewrite the baseline whenever a fix leaves stale entries.

## Configuration

Each file is governed by the nearest `.groovyfmt.toml` or `groovyfmt.toml` found by walking up from that file's own directory (with `--stdin-filepath`, from that path's directory), wherever groovyfmt runs from. `groovyfmt --check ../proj` uses `proj`'s config, and one run from a repository root formats a subproject with its own config by that config. A nested config replaces its parent's completely: nothing is merged, so repeat any settings the subproject should keep. The walk doesn't stop at the repository root, so a config in a directory above the checkout (a shared workspace directory, your home directory) applies too; `-v` shows which one is used. Files with no config above them use the defaults. `--config <file>` skips discovery and applies that one file to everything; `--indent` overrides the indent of whichever config applies. `--rules` and `--stdin` without `--stdin-filepath` use the config for the current directory. Everything is optional:

```toml
indent = 4                  # spaces per block level, 1 to 16
continuation_indent = 8     # arguments, literals, continued lines (default: same as indent)
use_tabs = false            # true: indent with tabs, one per indent level
indent_after_labels = false # true: indent Spock when:/then: blocks under their label
normalize_spacing = false   # true: also fix the spaces between tokens within a line
max_line_length = 120       # wrap longer lines where that is safe, 40 to 400 (default: no wrapping)
exclude = ["generated/**", "**/*.gsh"] # skip these files and directories
baseline = ".groovyfmt-baseline.json"   # lint reports only what isn't in it (see Baseline)

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

The action installs the release binary for the runner (Linux or macOS, x64 or arm64, or Windows x64), checks it against its `.sha256`, puts it on `PATH` and runs it with `args`:

```yaml
- uses: actions/checkout@v5
- uses: costajohnt/groovyfmt@v0.9.1
  # with:
  #   args: --check --output-format github   # the default; "" only installs
  #   version: 0.9.1                         # default: the release of the action ref used
  #   token: ${{ github.token }}             # the default
- run: groovyfmt --lint --output-format github
```

`--output-format github` turns findings into annotations on the pull request diff. While the repository is private, `token` must be able to read it, which `github.token` can only do in workflows of this repository. Without `version`, the action installs the release named by the `Cargo.toml` at the ref you use, so a release tag installs that release and `main` the latest one.

On a Windows runner the action's steps run in bash (Git Bash, which GitHub's Windows images have) and unpack the `.zip` with PowerShell's `Expand-Archive`; later steps find `groovyfmt` on `PATH` in `pwsh`, `cmd` or `bash`. This has not run on a GitHub Windows runner yet.

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

The Gradle plugin (`gradle-plugin/`) adds three tasks: `groovyfmtCheck` and `groovyfmtLint`, which `check` runs, and `groovyfmtFormat`, which formats in place. It downloads the release binary for the machine (Linux or macOS, x86_64 or arm64, or Windows x86_64) into `~/.gradle/groovyfmt/<version>/<target>/`, checks it against its `.sha256` before unpacking it, and reuses it after that. It needs Gradle 7.6 or later on Java 11 or later, and works with the configuration cache.

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
    // version = '0.9.1'                          // default: the plugin's own version
    // binary = file('/usr/local/bin/groovyfmt')  // run this binary and download nothing
    // args = ['--strict']                        // added to every run
    // paths = ['src', 'build.gradle']            // default: the project directory
}
```

Once it is published, drop `includeBuild` and write `id 'io.github.costajohnt.groovyfmt' version '<version>'`.

A task fails with groovyfmt's own output when files need formatting or lint finds something, and with a separate message when groovyfmt itself fails (exit code 2: bad arguments, a bad config, an unreadable file). The tasks are never up to date: groovyfmt finds its own files and configs, so Gradle can't tell when they changed.

Apply the plugin to the root project only. groovyfmt searches the whole directory tree, subprojects included (it skips `build` directories), so one run covers a multi-project build; applying it to subprojects too would check their files twice.

While the repository is private, the download needs a token that can read it: the environment variable `GROOVYFMT_GITHUB_TOKEN` or `GITHUB_TOKEN`, or the Gradle property `groovyfmt.githubToken` in a `gradle.properties` file. The token is sent only to `api.github.com`, in a header. Don't pass it as `-P` on the command line: Gradle 7.6 keeps every command-line `-P` value, in plain text, in its configuration cache. With `--offline` the plugin uses the cached binary and fails if there is none. On Windows on ARM, which has no release binary, set `binary`.

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
    rev: v0.9.1
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

## Using groovyfmt with coding agents

A coding agent that edits a Jenkinsfile or a Gradle script has no compiler to tell it whether the file still reads as Groovy. groovyfmt answers in milliseconds, in a form an agent can act on: exit codes, `path:line:col` messages, and JSON.

### Claude Code hook

This [PostToolUse hook](https://code.claude.com/docs/en/hooks) formats every Groovy file Claude writes or edits, then lints it. It goes in `.claude/settings.json` (or `~/.claude/settings.json`) and needs `jq` and `groovyfmt` on `PATH`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "f=$(jq -r '.tool_input.file_path // empty'); case \"$f\" in *.groovy|*.gradle|*.gvy|*.gy|*.gsh|*/Jenkinsfile|Jenkinsfile) groovyfmt -- \"$f\" >/dev/null 2>&1; groovyfmt --lint --strict --output-format json -- \"$f\" >&2 || exit 2;; esac"
          }
        ]
      }
    ]
  }
}
```

The hook reads the edited path from the `tool_input.file_path` field of its JSON input and does nothing for other files. It formats the file in place, which changes nothing when the file doesn't parse. Then it lints with `--strict`, so a file groovyfmt couldn't parse fails too. When anything is found, the JSON report (lint violations, or `ParseError` diagnostics with line and column) goes to stderr and the hook exits 2; for PostToolUse, Claude Code then shows that stderr to Claude next to the tool result. A clean file prints nothing to Claude. `exclude` in `.groovyfmt.toml` still applies, since the file is named on the command line.

### Instructions for the agent

Put this in `AGENTS.md` or `CLAUDE.md`:

```markdown
## Groovy, Gradle and Jenkinsfiles

After editing a `.groovy`, `.gradle` or `Jenkinsfile`, check it with groovyfmt:

- `groovyfmt --syntax <file>`: does it parse? Each problem prints as
  `file:line:col: could not parse: <message>`. groovyfmt's grammar is not the Groovy
  compiler, so a few valid files fail; if the code is right, say so instead of rewriting it.
- `groovyfmt --lint --fix <file>`: removes unused, duplicate and same-package imports and
  stray semicolons, then lists what is left. `groovyfmt --explain <Rule>` explains a rule.
- Fix the violations your change introduces. Don't add `groovyfmt-ignore` comments,
  `@SuppressWarnings` or baseline entries to silence them unless you are asked to; a
  baseline (`.groovyfmt-baseline.json`) already holds the old ones, so don't rewrite it.
- `groovyfmt --check --diff <file>`: shows what formatting would change; `groovyfmt <file>`
  applies it.
- Add `--output-format json` for a machine-readable report.

Exit codes: 0 clean, 1 something to fix (parse error, lint violation, or formatting),
2 a usage or I/O error.
```

### Language server

An agent or tool that speaks the Language Server Protocol can run `groovyfmt --lsp` instead of the CLI ([Editors](#editors)). It publishes parse errors and lint violations for each document it opens, with the same rules, config and positions as `--syntax` and `--lint`, returns formatting as a text edit that changes only whitespace, and offers the `--lint --fix` fixes as code actions. It never writes a file: the edits go back to the client, which applies them.

### pre-commit

To check the same things before each commit, use the hooks in [pre-commit](#pre-commit).

## Editors

`groovyfmt --lsp` is a language server on stdin and stdout. An editor that speaks the Language Server Protocol gets formatting (whole file and selection), parse errors and lint violations as you type, and the `--lint --fix` fixes as code actions ("Fix all auto-fixable problems", kind `source.fixAll.groovyfmt`, and a quick fix per fixable violation), all from the binary the CLI uses. Each document gets the config `--stdin-filepath` would find for its path, read again on every request, so an edited `.groovyfmt.toml` applies right away; an excluded file gets no diagnostics and no edits, and a buffer that doesn't parse gets its parse errors and no edits. `--config` and `--indent` apply as on the command line. It is a formatter and linter server, not a Groovy language server: no completion, hover, go-to-definition or rename. Only `file:` documents are served, so an unsaved buffer with no path gets nothing.

- **Neovim** 0.11 or later, with the built-in client:
  ```lua
  vim.lsp.config("groovyfmt", {
    cmd = { "groovyfmt", "--lsp" },
    filetypes = { "groovy" },
    root_markers = { ".groovyfmt.toml", "groovyfmt.toml", ".git" },
  })
  vim.lsp.enable("groovyfmt")
  ```
  `vim.lsp.buf.format()` formats; `vim.lsp.buf.code_action({ context = { only = { "source.fixAll" } }, apply = true })` applies the fixes.
- **VS Code** has no groovyfmt extension, and VS Code itself only runs language servers that an extension starts. A generic client such as [Generic LSP Client](https://github.com/llllvvuu/vscode-glspc) (`llllvvuu.glspc`) can start it, in `settings.json`:
  ```json
  "glspc.serverCommand": "groovyfmt",
  "glspc.serverCommandArguments": ["--lsp"],
  "glspc.languageId": "groovy"
  ```
  To fix on save, add `"editor.codeActionsOnSave": { "source.fixAll.groovyfmt": "explicit" }`.
- **Helix** (`languages.toml`; Helix knows Groovy, `*.gradle` and Jenkinsfiles but has no Groovy language server by default):
  ```toml
  [language-server.groovyfmt]
  command = "groovyfmt"
  args = ["--lsp"]

  [[language]]
  name = "groovy"
  language-servers = ["groovyfmt"]
  auto-format = true
  ```
- **Zed** can't add a language server from settings alone; it starts the ones its extensions declare. With the [Groovy extension](https://github.com/valentinegb/zed-groovy), which declares a server named `groovy`, point that server at groovyfmt in `settings.json` (this replaces the extension's own server):
  ```json
  "lsp": {
    "groovy": { "binary": { "path": "/usr/local/bin/groovyfmt", "arguments": ["--lsp"] } }
  },
  "languages": {
    "Groovy": { "formatter": "language_server", "format_on_save": "on" }
  }
  ```

Without a language server client, call `groovyfmt --stdin`, which prints nothing and exits 2 when it can't format the buffer, so a parse error never replaces your code.

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
