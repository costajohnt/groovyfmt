# Changelog

## 0.4.0 (2026-11-09)

From a full audit (`docs/audit-2026-11-09.md`). groovyfmt runs in CI on pull-request content, so these matter:

### Security

- Symlinks inside searched directories are no longer followed. A link cycle used to hang the run, and a link out of the tree could make groovyfmt read or rewrite files elsewhere.
- A file name starting with `::` can no longer inject a GitHub Actions workflow command into the log.
- Deeply nested input no longer crashes `--lint` with a stack overflow, and no longer makes the formatter's output grow quadratically.
- FIFOs, device files and files over 10 MB are skipped.
- In-place writes are atomic (temporary file plus rename, permissions kept), so a crash can't leave a truncated file, and a file swapped for a symlink mid-run is replaced rather than written through. Read-only files are left alone.

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
