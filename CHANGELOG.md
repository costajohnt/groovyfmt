# Changelog

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
