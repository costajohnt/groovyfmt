# groovyfmt

A fast formatter for Groovy, Gradle build scripts (`build.gradle`), and Jenkinsfiles, written in Rust on a tree-sitter grammar.

Status: early. The formatter changes whitespace only; the linter has five rules.

```sh
groovyfmt <files or dirs>          # format in place
groovyfmt --check <files or dirs>  # list files that would change; exit 1 if any
groovyfmt --stats <dirs>           # corpus report: unchanged/formatted/skipped, idempotence, throughput
groovyfmt --lint <files or dirs>   # lint; prints path:line:col: Rule: message, exit 1 if any
```

Files are processed in parallel on all cores.

## Lint rules

Named after their CodeNarc equivalents so existing configs map across:

| Rule | Flags |
|---|---|
| UnusedImport | import whose name (or alias) never appears in code or comments |
| DuplicateImport | the same import twice |
| UnnecessaryGString | double-quoted string with nothing to interpolate |
| UnnecessarySemicolon | `;` at the end of a line |
| EmptyCatchBlock | empty `catch`, unless the parameter is named `ignore` / `ignored` |

Files that don't parse cleanly are not linted, to avoid false positives.

What v0 does:

- re-indents every line with 4-space steps from the parse tree (blocks, closures, argument lists, literals, continued expressions, Spock `when:`/`then:` labels)
- strips trailing whitespace, collapses runs of blank lines to one, ends the file with one newline
- leaves lines inside multi-line strings and comments untouched

Safety: the output is re-parsed and its token stream compared with the input's. If anything other than whitespace differs, or the input doesn't parse cleanly, the file is skipped and left as is.

## Corpus numbers (2026-11-08, 4-core claude-worker)

23,795 files from grails-core, groovy, gradle, spock, CodeNarc, nextflow and Jenkins pipeline repos:

| | files |
|---|---|
| parse cleanly | 19,244 (80.9%) |
| already formatted | 12,449 |
| reformatted | 6,795 |
| changed again by a second pass | 0 |
| skipped, parse error | 4,551 |

Timing on the 119 MB corpus: format with idempotence check 8.8s (13.5 MB/s), lint 4.7s. Spot check: 10 of 10 sampled UnusedImport findings were real (the symbol appears only on the import line). UnnecessaryGString is noisy on codebases that use double quotes by convention (81k of its 104k corpus hits are in gradle/gradle).

## Grammar

groovyfmt builds on [dekobon/tree-sitter-groovy](https://github.com/dekobon/tree-sitter-groovy), pinned to the fork branch `costajohnt/tree-sitter-groovy@groovyfmt-base` until upstream catches up. Files parsing cleanly on the corpus:

| grammar | clean |
|---|---|
| dekobon 0.3.0 (crates.io) | 63.8% |
| + upstream PR #49 | 75.0% |
| + `final` locals and named-argument command chains (our branches) | 80.9% |

Neither step makes any previously clean file fail. The other grammars scored lower (murtaza64 47.8%, amaanq 18.7%); build them with `--no-default-features --features g-murtaza` (or `g-amaanq`).

## Known gaps

- Grammar coverage is the limit: about 19% of real Groovy files still hit parse errors and are skipped.
- The grammar doesn't always end a declaration at a newline: `private task` followed by `Foo(a) {}` on the next line parses as one method, which mis-indents what follows. Tokens are unchanged; only the indentation is wrong.
- No config file, no stdin mode, no line wrapping or spacing inside lines yet.
