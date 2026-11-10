# Changelog

## Unreleased

### Added

- `--stdin-filepath <path>` (with `--stdin`): find the config file by walking up from that path's directory, as if formatting the file on disk, and use the path as the name in error messages and `--output-format github` annotations. The file need not exist. If the path matches `exclude`, stdin is written back unchanged and the exit status is 0.
- `exclude = [...]` config option: glob patterns, relative to the config file's directory, for files and directories to skip on top of the built-in list. `*` and `?` match within a path segment, `**` across segments. Excluded directories are not descended into, and excludes also apply to files named on the command line.

### Fixed

From the second audit (`docs/audit-2026-11-09-full.md`):

- File names and lint messages can no longer inject the older `##[command]` GitHub Actions workflow syntax, which the runner accepts anywhere in a line (`##[add-mask]`, `##[stop-commands]`, `##[error]`). Every printed path, message and panic report shows `##[` as `##\[`.
- Formatting a run of indented comments was cubic in its length (4,000 lines took over two minutes) and long blocks were superlinear. The indent pass now moves one tree cursor forward through the file instead of looking up each line's node, parents and previous siblings from the root: 4,000 comment lines take 0.01 s, and an 800,000-line file 5.7 s instead of 30 s.
- In-place formatting and `--check` no longer hold every changed file's new text in memory until the run ends. Each worker writes the file, or for `--check` only notes that it changed, as soon as it is formatted.
- A file named more than once, directly or through overlapping arguments (`groovyfmt . src`, `A.groovy ./A.groovy`, `sub/../A.groovy`), is formatted and reported once. Each in-place write uses its own temporary file name, and a failed write no longer deletes a file that already had that name.
- UnusedImport no longer reports a class used only through a brace-less GString interpolation such as `"$Name"` or `"$Name.CONST"`; removing the import as advised broke the code.
- Formatting a file in place keeps its owner and group (root formatting a bind-mounted checkout used to leave every changed file owned by root) and keeps its hard links: a file with more than one link, or whose owner can't be kept, is overwritten in place instead of replaced by a renamed temporary file. A path swapped for a symlink or anything else between the walk and the write is now left alone with an error, where it used to be replaced.

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
