# groovyfmt

A fast formatter for Groovy, Gradle build scripts (`build.gradle`), and Jenkinsfiles, written in Rust on a tree-sitter grammar.

Status: early. v0 changes whitespace only.

```sh
groovyfmt <files or dirs>          # format in place
groovyfmt --check <files or dirs>  # list files that would change; exit 1 if any
groovyfmt --stats <dirs>           # corpus report: unchanged/formatted/skipped, idempotence, throughput
```

What v0 does:

- re-indents every line with 4-space steps from the parse tree (blocks, closures, argument lists, literals, continued expressions, Spock `when:`/`then:` labels)
- strips trailing whitespace, collapses runs of blank lines to one, ends the file with one newline
- leaves lines inside multi-line strings and comments untouched

Safety: the output is re-parsed and its token stream compared with the input's. If anything other than whitespace differs, or the input doesn't parse cleanly, the file is skipped and left as is.

## Corpus numbers (2026-11-08)

23,795 files from grails-core, groovy, gradle, spock, CodeNarc, nextflow and Jenkins pipeline repos:

| | files |
|---|---|
| parse cleanly (dekobon grammar) | 15,174 (63.8%) |
| already formatted | 10,214 |
| reformatted | 4,960 |
| changed again by a second pass | 0 |
| skipped, parse error | 8,621 |

Grammar bake-off, files parsing cleanly: dekobon-tree-sitter-groovy 63.8%, murtaza64/tree-sitter-groovy 47.8%, tree-sitter-groovy (amaanq) 18.7%. Build another grammar with `--no-default-features --features g-murtaza` (or `g-amaanq`).

## Known gaps

- Grammar coverage is the limit: about 36% of real Groovy files still hit parse errors and are skipped.
- The grammar doesn't always end a declaration at a newline: `private task` followed by `Foo(a) {}` on the next line parses as one method, which mis-indents what follows. Tokens are unchanged; only the indentation is wrong.
- No config file, no stdin mode, no line wrapping or spacing inside lines yet.
