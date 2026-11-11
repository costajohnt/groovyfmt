# Changelog

## Unreleased

### Added

- pre-commit hooks in the repository (`repo: https://github.com/costajohnt/groovyfmt`): `groovyfmt`, `groovyfmt-check` and `groovyfmt-lint` run the binary on `PATH`, and `groovyfmt-source`, `groovyfmt-check-source` and `groovyfmt-lint-source` build it from source with cargo.
- A GitHub Action (`uses: costajohnt/groovyfmt@<ref>`): installs the release binary for the runner (Linux or macOS, x64 or arm64) after checking its `.sha256`, puts it on `PATH`, and runs it with `args` (default `--check --output-format github`).
- Packaging for npm (`npm/`, the `groovyfmt` package with one `@groovyfmt/<os>-<arch>` package per platform), Homebrew (`packaging/homebrew/`, a formula rendered from a release's checksums) and a Docker image (`Dockerfile`, `scratch` plus the static binary). None of these is published yet: the release workflow's `publish-npm` and `publish-image` jobs stay off until the repository variables `PUBLISH_NPM` and `PUBLISH_IMAGE` are set (`docs/releasing.md`). CI builds and runs each against the latest release.
- `--strict`: with `--check`, `--lint` or when formatting in place, list the files that were skipped (they don't parse, aren't UTF-8, are over 10 MB or nest too deeply) and exit 1 if there are any. Without it a skipped file never affects the exit status, so CI can pass on files nothing checked.
- `--` ends the options: every argument after it is a path, even one starting with `-`.
- `--syntax` only parses, and reports each place groovyfmt could not parse as `path:line:col: could not parse: <message>` (up to 10 per file, then a count of the rest), exiting 1 if any file doesn't parse. The message names a bracket left open, or quotes the text the grammar stopped at and where it was. `-v` and `--strict` print the same lines under each file skipped for a parse error, and `--stdin` prints them after its error.
- `--output-format json` for `--check`, `--lint`, `--syntax` and formatting in place: one JSON document on stdout with each file's status, skip reason and diagnostics (with end positions) and a summary, in a versioned schema documented in the README. `--output-format sarif` for `--lint` and `--syntax`: a SARIF 2.1.0 log for GitHub code scanning. File names and quoted source are escaped so they can't act as terminal escapes or workflow commands.
- `--lint --fix` removes unused and duplicate import lines and unnecessary semicolons in place, then reports what remains (exit 1 if anything does). Each fix is checked as formatting is: the result must parse cleanly, keep every other token and the shape of the parse tree, and lint with no new violations, or the file is left untouched and listed as `not fixed`. Files that don't parse are never fixed; writes follow the same symlink, owner, hard-link and `exclude` rules as formatting.
- A Gradle plugin (`gradle-plugin/`, id `io.github.costajohnt.groovyfmt`) with `groovyfmtCheck` and `groovyfmtLint`, which `check` runs, and `groovyfmtFormat`. It downloads the release binary into `~/.gradle/groovyfmt/`, checks its `.sha256` before unpacking it, and reuses it; `binary` runs a local one instead. Gradle 7.6+ on Java 11+, configuration cache compatible. Not published yet: use it with `includeBuild` from a checkout (README, Gradle). The release workflow's `publish-gradle-plugin` job stays off until the repository variable `PUBLISH_GRADLE_PLUGIN` is set (`docs/releasing.md`).

- Lint rules for declarative Jenkinsfiles, Gradle build scripts and Spock specs, for the structural mistakes those tools otherwise report only when a build runs (README, Lint rules). Jenkins: `JenkinsMissingSection`, `JenkinsConflictingSections`, `JenkinsUnknownSection`, `JenkinsStatementOutsideSection`, `JenkinsScriptRequired`, `JenkinsUnknownWhenCondition`, `JenkinsMisplacedOption`, `JenkinsDuplicateStageName`, `JenkinsStageName` and `JenkinsAgentForm`, checked on a top-level `pipeline { }` in `Jenkinsfile`, `Jenkinsfile.*`, `*.jenkinsfile` or any file that starts with one. Gradle (`*.gradle`): `GradleRemovedConfiguration`, `GradleJcenter`, `GradleLeftShiftTask` and `GradleDynamicVersion` (off by default). Spock: `SpockBlockOrder`, `SpockDataTableColumns` and `SpockEmptyBlock` (off by default). On the pipeline-model-definition plugin's own test pipelines the Jenkins rules report nothing on the 283 valid ones and catch 49 of the 132 invalid ones (49 of the 55 whose error is the kind these rules look for; the rest are step parameter, environment value and matrix axis errors, which need the installed plugins to judge). They report no false positives on about 300 other real pipelines, and the Spock rules that are on by default report nothing on Spock's own specs.

### Changed

- `--lint` text output ends each violation `--lint --fix` would fix with ` (fixable)` (`UnusedImport`, `DuplicateImport` and `UnnecessarySemicolon`; an import sharing its line with anything but a `;` isn't fixable). The `github` format is unchanged.
- The grammar is vendored: its generated C sources live in `grammar/` (from the tree-sitter-groovy fork at the same commit as before, `65a9b34`) and `build.rs` compiles them, replacing the git dependency on the fork. groovyfmt now builds from this repository alone, so `cargo install --locked --git` fetches one repository and the crate can be packaged for crates.io. Output is unchanged: formatting and `--lint` give byte-identical results on the 23,766-file corpus and 78 trial repositories. `scripts/update-grammar.sh` updates `grammar/` from a grammar checkout.
- The grammar pin moves to a newer `groovyfmt-base` of the tree-sitter-groovy fork. Files skipped for a parse error go from 117 to 85 on the 23,766-file corpus, from 21 to 15 across the 13 repositories, from 26 to 23 across the 21 more, and from 48 to 6 across 41 more repositories (5,029 files). Newly accepted: `def` before a class or interface (`def class X`, `def static class X`), `def final x = 1` and `def Object x` parameters; `new`, `switch`, `in` and `static` where they are names (`[switch: 'on']`, `in.property`, `static.unused = { }`, `yield()`); a comparison right after a name where type arguments could start (`if ((value>0)&&(value<99))`, `i<n ? a : b`); slashy strings with `\\/` (`~/:\\/\\/.+/`); quoted command names (`'x:title' lang: 'en', 'text'`, `'CatchThrowable' priority: 1`); a call chained onto a call (`a(x) or(b(y))`, `(page.list 0 isEmpty())`); `assertEquals (-1), x`, `visibilityOf(el modalBy)` and `def api = toApi 'A': src`; Groovy 5 and 6 forms (`for await`, `for (int i, v in list)`, `case Integer i when i > 0 ->`, `??`, `??=`); spaced and annotated type arguments (`(List <T>) x`, `Closure <V> v)`, `Arrays. < T > asList(a)`, `String @A []`, `implements @A I`); nested array initializers and `@Target({A, B})`; script variables with modifiers (`public static final long X = 1`), `Object a = f(), b`, `def (head, *tail) = l`, `(1..5)[*1..3]`, `[][]`; `assert` and labels inside `${}`; files starting with two byte order marks; `x .42`; and `else` is now always a keyword, so `if (a) a b` followed by `else` on the next line keeps its `else`.
- Each thread reuses one parser instead of creating one per parse (two to four per file): about 2% less CPU time on the 23,766-file corpus (`--stats`, 44.4 s user against 45.2 s, mean of 5 runs each).

### Fixed

- `UnusedImport` no longer reports an annotated import (`@Library('lib') import x`, `@Grab('g:a:v') import y`): the import is there to carry the annotation, which loads the library. A duplicate annotated import is still reported but not fixable. Across the corpus and the 78 trial repository directories this removes 1 report.
- `UnnecessarySemicolon` no longer reports a `;` before a line starting with `++` or `--`: without it, `a[0]` then `++n` reads as `a[0]++` then `n` (bigtop `TestBlockRecovery`). This removes 1 report across the same code.
- A body's `{` on a line of its own (`void f()` then `{`, the Allman style) lines up with its declaration or statement, as its `}` and its first statement already did, instead of being indented as a continuation. Of the files that parsed cleanly before, 657 across the corpus and the 75 repositories change, every changed line one of those `{` lines moving out one step.
- A line starting with a map key or named argument called `else`, `catch` or `finally` (`[` then `else: 1,`) keeps its indent instead of moving to the level of the line holding the `[` or `(`. Across the corpus and the 34 trial repositories no file has that shape; one GPars file changes because the grammar reads its `else` (after `if (follower) follower value`) as a name, and that `else` block now lines up with its `if` instead of sitting one level left of it.
- Formatting a file whose name is close to the 255-byte limit no longer fails with `File name too long`: the temporary file beside it now has a short name of its own instead of one built from the file's name.
- A worker thread that dies outside the per-file panic guard now turns each of its files into an `internal error, please report` error. Before, its results were dropped, the list came back short and later results could be reported against the wrong files.
- Command-line parsing: a flag that takes no value rejects one (`--check=false` used to turn check on), and `--stdin` with path arguments is an error instead of ignoring the paths (name the buffer with `--stdin-filepath`).
- `UnusedImport` no longer reports a class used only inside `${...}` that the grammar leaves as plain string text, as in a `"""` string holding `"${TextUtil.escapeString(path)}"` (common in Gradle's integration tests). Across the corpus and the 34 repositories this removes 97 false reports.
- `UnusedImport` checks comments for the name as a whole word, so `// Listing` no longer counts as a use of `List`. This adds 29 reports across the corpus and the 34 repositories, each an import whose name appears nowhere else as a word (`import org.gradle.api.Project` next to `ProjectBuilder`). Comment text used to be searched once per import, which took 10 s on a 5 MB file with 30,000 imports; it now takes under 1 s.
- `UnnecessarySemicolon` no longer reports a `;` before a line starting with `{`: without it the closure on that line becomes an argument of the statement before (`new DataflowVariable();` then `{-> ... }.callTimeoutAsync(t)` in GPars).
- `AssignmentInConditional` treats an assignment in extra parentheses, `if ((m = matcher(s)))`, as meant, as GCC and ESLint do.
- `ComparisonWithSelf` (off by default) no longer reports a comparison inside a GORM where query (`Book.where { title == title }`, `@Where({ ... })`), where the left side is the domain property and the right a variable.
- `DuplicateCaseStatement` treats `case 'a':` and `case "a":` as the same label, as `DuplicateMapKey` already did for keys.

## 0.8.0 (2026-11-10)

### Changed

- The grammar pin moves to a newer `groovyfmt-base` of the tree-sitter-groovy fork. On the 23,766-file corpus, files skipped for a parse error go from 316 to 117; across 13 open source repositories outside the corpus, from 137 to 21; across 21 more repositories (5,938 files), from 134 to 26. Newly accepted: `var`, `pipeline` and `trait` as names (`var.set(1)`, `{ var -> }`, `Pipeline pipeline = p`, `pipeline.stages << s`), generic types as values (`Mock(Supplier<Long>)`, `ArrayList<Job>.class`) and after a space in declarations (`Map <String, X> m = [:]`), dotted generic and array types after `as`, command calls whose first argument starts with `new`, `null`, `!`, `~` or a dotted name or is an operator expression or list (`respond new Foo(), model: m`, `println it.name`, `assertTrue x instanceof Y, 'msg'`, `assertEquals [:], m`), command calls in parentheses (`(el searchBy).click()`, `return (run service: 'x')`), anonymous class bodies and instance initializers, `instanceof` pattern variables, slashy strings starting with a space or spanning lines, `\` line continuations, `"\${x}"` escapes in shell text inside `"""` strings, annotated, `final` and `def`-typed closure parameters, `default` methods, annotated quoted method names (`@Unroll 'a feature'() { }`), script methods with modifiers and no return type (`private deploy(script) { }`), annotations in type arguments and after `extends` and `new`, annotated enum constants, `map.(key)`, `final def (a, b)`, `catch (final E e)`, `try (parser)`, `while (--x);`, `... args`, `.5`, Unicode identifiers and `[*  : m]`.
- Some files that already parsed now format differently because the grammar reads them more accurately. An anonymous class body (`new Foo(a,` then `b) {`) is indented from the line the `new` starts on, like a class body, instead of from the line of its `{`; an `else` after `if (c) reply m.reverse()` lines up with its `if` instead of being read as a statement named `else`; annotated closure parameters and enum constants, and a statement after `x as a.b.C<T>`, are no longer indented as continuations. Of the files that parsed cleanly before, 23 across the corpus and the 13 repositories (and 2 in the 21 more) now format differently: 33 lines move in and 74 move out, all in those cases. Files that parsed cleanly before still parse.

### Fixed

- `UnnecessarySemicolon` no longer reports the `;` that is an empty `while` body (`while (it.next());`), which the grammar now parses: removing it would make the next statement the loop body.

## 0.7.0 (2026-11-10)

### Changed

- The grammar pin moves to a newer `groovyfmt-base` of the tree-sitter-groovy fork. On the 23,766-file corpus, files skipped for a parse error go from 930 to 316; across 13 open source repositories outside the corpus, from 508 to 137. Newly accepted: array types as values (`byte[].class`, `getBean(String[], x)`, `String[]::new`), a command call as the value of a declaration or assignment (`def r = sh returnStdout: true, script: 'x'`, `x = foo bar`), named arguments on the line after a comma, `//` and `/*` inside a GString, statements in a GString interpolation (`${x.replace 'a', 'b'}`, `${-> x}`), annotations after modifiers (`static @Rule TestName name`, `final @Delegate Map m`), `record` as a name, `def String name`, uninitialized `List<String> x` locals, closure parameter defaults and varargs, `private "a feature"()`, `f(*: m)`, `map['a', 'b']`, `public @interface`, `enum E implements I`, enum constant bodies, `case 1, 2 ->`, and generic types as annotation arguments.
- Some files that already parsed now format differently because the grammar reads them more accurately. A line that starts with a name or `(` after a complete statement starts a new statement: `x` followed by `bar = 1` used to be read as one declaration typed by `x`, and `if (c) return` took the next line as its value, so the second line was indented as a continuation (`classes.setFrom preparePluginValidationClasses` followed by `enableStricterValidation = true` did this). Fields and comments after an uninitialized typed field are no longer indented as continuations either, and an `else` after `if (c) a.b key` lines up with its `if`. Of the files that parsed cleanly before, 513 across the corpus and the 13 repositories now format differently: 2,826 lines move out to the level of their block, and 3 lines (two `else` lines and one `else` body) move in under their `if`. No statement is indented as a continuation. Files that parsed cleanly before still parse.

## 0.6.0 (2026-11-10)

### Added

- `use_tabs` config option: indent with tabs instead of spaces. Indentation is still worked out in columns, with a tab `indent` columns wide, and each full tab width is written as a tab, so `continuation_indent` keeps its meaning (the default gives continued lines one extra tab; `continuation_indent = 8` with `indent = 4` gives two). On spring-boot, which indents with tabs, it cuts the first-run diff from 760 files to 97. Space-indented output is unchanged.
- `--suggest-style`: format the files in memory under six indent styles (the defaults, `indent = 2`, `use_tabs = true`, `continuation_indent = 8`, and two combinations), print the config lines for the one that leaves the most files unchanged (a tie goes to the one changing fewer lines, then to the defaults, then to the simpler style), and show what each would change. Nothing is written. On ratpack it suggests `indent = 2`, on spring-boot `use_tabs = true`, on OpenBoxes `continuation_indent = 8`, and on geb it reports that the defaults fit.

### Changed

- The grammar pin moves to a newer `groovyfmt-base` of the tree-sitter-groovy fork, which parses many more files. On the 23,766-file corpus, files skipped for a parse error go from 2,827 to 930; across 13 open source repositories outside the corpus, from 1,488 to 508. Newly accepted: annotated script and field declarations (`@Field String NAME = 'x'`, Spock's `@Shared log = []`), parenthesis-free calls with several arguments, quoted named-argument keys or a dotted receiver (`bind Foo, Bar`, `attributes 'Main-Class': m`, `script.writeFile file: f, text: t`), array creation (`new byte[10]`, `new String[] { 'a' }`), Java lambdas, nested annotations as annotation values, typed and def-less multiple assignment, `assert e, message`, `def static`, `def <T> T f()`, bodiless untyped methods, annotation element defaults, `synchronized` blocks, `?.@` and `*.@`, `[*a, *b]`, `System::'print'`, `Collections.<String>emptyList()`, and annotations on `package` and `import`.
- Some files that already parsed now format differently because the grammar reads them more accurately. A `(` or a string on the line after an expression starts a new statement instead of continuing it (`println 'x'` followed by `(1..n).each { }`), so the next line is no longer indented as a continuation. Files that parsed cleanly before still parse. Statement keywords can no longer be read as names, so `List ids = (List) items.ids` followed by `if (ids) {` keeps the `if` at statement level, and a cast's value must start on the line of its `)`, so `x = (e.user)` followed by a statement on the next line no longer indents that statement as a continuation.

## 0.5.0 (2026-11-10)

Most of this release comes from a second full audit (`docs/audit-2026-11-09-full.md`). groovyfmt runs in CI on pull-request content, so these matter:

### Security

- File names and lint messages can no longer inject the older `##[command]` GitHub Actions workflow syntax, which the runner accepts anywhere in a line (`##[add-mask]`, `##[stop-commands]`, `##[error]`). Every printed path, message and panic report shows `##[` as `##\[`.
- A symlink named on the command line stays a symlink: the file it points to is formatted, through any chain of links, and reports name the link as typed. Before, the link was replaced by a formatted regular file and its target left untouched. A link whose target matches `exclude` is skipped, and in write mode a named link, to a file or a directory, whose target is outside the working directory is an error (exit 2) and nothing is rewritten through it, so `groovyfmt *` in an untrusted checkout can't write through a committed link to files elsewhere; `--check` and `--lint` still read such a link. Arguments that are not themselves links, such as `../other-project`, are unaffected.
- A file is only written if its path still names the file that was read. If it was replaced by another file or a symlink in between, the write fails with `file changed while formatting` (exit 2) and nothing is written; before, the rename replaced whatever was at the path. Edits made to the same file in between are not detected.
- Formatting a run of indented comments was cubic in its length (4,000 lines took over two minutes) and long blocks were superlinear. The indent pass now moves one tree cursor forward through the file instead of looking up each line's node, parents and previous siblings from the root: 4,000 comment lines take 0.01 s, and an 800,000-line file 5.7 s instead of 30 s.
- In-place formatting and `--check` no longer hold every changed file's new text in memory until the run ends. Each worker writes the file, or for `--check` only notes that it changed, as soon as it is formatted.

### Added

- `--stdin-filepath <path>` (with `--stdin`): find the config file by walking up from that path's directory, as if formatting the file on disk, and use the path as the name in error messages and `--output-format github` annotations. The file need not exist. If the path matches `exclude`, stdin is written back unchanged and the exit status is 0.
- `exclude = [...]` config option: glob patterns, relative to the config file's directory, for files and directories to skip on top of the built-in list. `*` and `?` match within a path segment, `**` across segments. Excluded directories are not descended into, and excludes also apply to files named on the command line.

### Changed

- Each file is now governed by the nearest `.groovyfmt.toml` / `groovyfmt.toml` above the file itself, not the one above the working directory. `groovyfmt --check ../proj` used to apply the working directory's config (or the defaults) to `proj` and report files formatted under `proj`'s own config as needing changes, and write mode re-indented them; a subproject with its own config was formatted by the root's config when run from the root. A nested config replaces its parent's entirely, nothing is merged. Its `exclude` and `[lint]` settings apply inside its directory, and the parent's `exclude` still decides whether that directory is searched (a config inside an excluded directory is never read). That parent exclusion also covers files named on the command line, `--stdin-filepath`, and runs started inside the excluded directory, which used to see only the nested config; `--config` overrides it. A file outside the working directory no longer picks up the working directory's config. `--config` still applies one file to everything and `--indent` still overrides the indent; `--rules` and `--stdin` without `--stdin-filepath` still use the working directory's config. A config file that fails to parse stops the run (exit 2) only when a file or directory it governs is reached. `-v` prints one `config:` line per config in use. (M5)
- The Linux arm64 release binary is now a static musl build, like the x86_64 one, so it runs without glibc (Alpine, Amazon Linux 2, RHEL 8, Debian 11, Ubuntu 20.04). Its asset is now named `groovyfmt-<tag>-aarch64-unknown-linux-musl.tar.gz` instead of `...-aarch64-unknown-linux-gnu.tar.gz`.
- The minimum supported Rust version is 1.90 (`rust-version` said 1.85, which can't build the tree-sitter dependencies). CI now runs the tests on 1.90.0.
- A release tag now publishes only if it matches the Cargo.toml version, is on `main`, has a CHANGELOG heading, and fmt, clippy, the tests and a build and smoke test of every release binary pass on the tagged commit. All four release targets are also built and run on every pull request.

### Fixed

- UnusedImport no longer reports a class used only through a brace-less GString interpolation such as `"$Name"` or `"$Name.CONST"`; removing the import as advised broke the code.
- Formatting a file in place keeps its owner and group (root formatting a bind-mounted checkout used to leave every changed file owned by root) and keeps its hard links: a file with more than one link, or whose owner can't be kept, is overwritten in place instead of replaced by a renamed temporary file.
- A file named more than once, directly or through overlapping arguments (`groovyfmt . src`, `A.groovy ./A.groovy`, `sub/../A.groovy`), is formatted and reported once. Each in-place write uses its own temporary file name, and a failed write no longer deletes a file that already had that name.

## 0.4.0 (2026-11-09)

From a full audit (`docs/audit-2026-11-09.md`). groovyfmt runs in CI on pull-request content, so these matter:

### Security

- Symlinks inside searched directories are no longer followed. A link cycle used to hang the run, and a link out of the tree could make groovyfmt read or rewrite files elsewhere.
- A file name starting with `::` can no longer inject a GitHub Actions workflow command into the log.
- Deeply nested input no longer crashes `--lint` with a stack overflow, and no longer makes the formatter's output grow quadratically.
- FIFOs, device files and files over 10 MB are skipped.
- In-place writes are atomic (temporary file plus rename, permissions kept), so a crash can't leave a truncated file, and a file swapped for a symlink mid-run is replaced rather than written through. Read-only files are left alone.

### Added

- `continuation_indent` config option: the indent for arguments, list and map literals, and continued lines, separate from block indentation. Set it to 8 for IntelliJ-formatted code.

### Parsing

- A `[` that starts a new line begins a list or map instead of subscripting the line above, so Grails controller actions ending in `[model: value]` parse. Also `final` method parameters, traits with `implements`, and subscripts as later arguments of parenthesis-free calls. Corpus coverage 85.6% to 88.1%; OpenBoxes (a Grails app outside the corpus) 88% to 95.6%.

### Fixed

- `indent_after_labels` was cubic in block length (221 s for a 20,000-line block, now 0.24 s), and treated any label, not just Spock's, as opening a block.
- Mixed line endings are kept per line instead of converting the whole file to CRLF.
- Lint columns count characters, not bytes, so annotations land in the right place after non-ASCII text.
- A crash while processing one file is reported as an error for that file (exit 2) instead of aborting the run; `| head` no longer panics; unreadable directories exit 2.
- The formatter's safety check now compares string fragments exactly, closing a blind spot for whitespace before `${…}` inside GStrings.

## 0.3.0 (2026-11-09)

### Added

- `indent_after_labels` config option: indent the statements of a Spock `given:` / `when:` / `then:` block one level under the label. grails-core uses this style in about half its specs; the default stays level with the label.

### Fixed

- An annotation followed by a trailing comment (`@Requires(x) // why`) no longer pushes the declaration and its body one level right.
- A `{` body whose declaration or statement wraps onto continuation lines (multi-line parameters, conditions, multi-catch) is indented from the line the declaration starts on.
- `//` comments at column 0 stay there, as IntelliJ does (commented-out code, doc-snippet tag markers).
- A file that isn't valid UTF-8 is skipped with a message instead of failing the whole run.
- Empty and whitespace-only files format to an empty file instead of a single newline.

## 0.2.0 (2026-11-09)

### Added

- Eleven lint rules from CodeNarc's basic set: EmptyIfStatement, EmptyElseBlock, EmptyTryBlock, EmptyFinallyBlock, EmptyWhileStatement, EmptyForStatement, DuplicateMapKey, DuplicateCaseStatement, AssignmentInConditional, ConstantIfExpression, and ComparisonWithSelf (off by default).
- Nested type declarations (`static class`, member `interface`, `trait`, `record`, `@interface`) now parse: corpus coverage 80.9% to 85.6%.

## 0.1.0 (2026-11-09)

First release: whitespace-only formatter with a token-equality safety check, `--check`, `--stdin`, `--lint` with five rules, `.groovyfmt.toml`, GitHub Actions annotations, and prebuilt binaries for Linux and macOS.
