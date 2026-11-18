# Coverage

groovyfmt only touches files it can parse.

- On a corpus of 23,766 files from grails-core, groovy, gradle, spock, CodeNarc, nextflow and Jenkins pipeline repositories, 99.6% parse cleanly (85 files are skipped).
- On [OpenBoxes](https://github.com/openboxes/openboxes), a production Grails application that isn't in the corpus, 99.5% of its 1,327 files do.

## What still fails

Of the 85 corpus files still skipped, 80 aren't Groovy source:

- 70 Grails, Gradle and CodeNarc templates with `@artifact.package@`, `<% %>` or `${className}` placeholders
- 9 Spock AST snapshots
- 1 deliberately broken groovydoc fixture

The other 5 use forms the grammar doesn't parse yet:

- Groovy 5 list patterns (`case [var only] ->`) and map destructuring (`def (from: a, to: b) = r`)
- a guard whose last operand is a bare name before `->`
- a multiple assignment and comma-separated updates in a `for` header
- a spread first command argument (`bridgedClasses *list, 'x'`)

## The grammar

The grammar is [dekobon/tree-sitter-groovy](https://github.com/dekobon/tree-sitter-groovy) as extended on the fork branch [`costajohnt/tree-sitter-groovy@groovyfmt-base`](https://github.com/costajohnt/tree-sitter-groovy/tree/groovyfmt-base), which adds upstream PR #49 plus groovyfmt's fixes to the 0.3.0 release. Its generated parser is vendored in [`grammar/`](../grammar/README.md), which records the fork commit it was copied from.

How each step changed the share of corpus files that parse cleanly:

| grammar | files parsing cleanly |
|---|---|
| dekobon 0.3.0 | 63.8% |
| + PR #49 | 75.0% |
| + #56 (`final` locals) and #57 (named-argument calls) | 80.9% |
| + nested type declarations (`static class`, member `interface`, …) | 85.6% |
| + a `[` starting a new line is read as a list or map (Grails controllers returning `[model: …]`); `final` parameters; traits with `implements` | 88.1% |
| + annotated script and field declarations (`@Field`, `@Shared`); command calls with several arguments, quoted keys or a dotted receiver; array creation; lambdas; nested annotations; multiple assignment; `def static`, `def <T>`, `synchronized` and smaller forms | 96.1% |
| + array types as values (`String[].class`); command calls as declaration and assignment values (`def r = sh returnStdout: true, script: 'x'`); a name or `(` on a new line starts a statement; `record` as a name; annotations after modifiers; statements in GString interpolations; `//` inside GStrings; closure parameter defaults and smaller forms | 98.7% |
| + `var`, `pipeline` and `trait` as names; generic types as values (`Mock(Supplier<Long>)`); command calls with `new`, `null`, `!` or dotted first arguments and in parentheses (`(el x).click()`); anonymous class bodies; `instanceof` pattern variables; slashy strings starting with a space; `\` line continuations; annotated and `final` closure parameters, `default` methods and smaller forms | 99.5% |
| + Groovy 5 and 6 forms (`for await`, `case Integer i when`, indexed `for (i, x in xs)`, `??`); `\\/` in slashy strings; a comparison right after a name (`i<n ? a : b`); quoted command names (`'x:title' lang: 'en'`) and calls chained onto calls; `new`, `switch`, `in` and `static` as names; nested array initializers and `{a, b}` annotation arrays; spaced and annotated type arguments and smaller forms | 99.6% |

[Development](development.md#changing-the-grammar) explains how to change the grammar.

## Speed

Against [npm-groovy-lint](https://github.com/nvuillam/npm-groovy-lint) 19.0.1 (CodeNarc 3.7.0) on the [spock](https://github.com/spockframework/spock) repository, 635 files, 4-core Linux box:

| task | npm-groovy-lint | groovyfmt |
|---|---|---|
| lint, the same four rules | 19.5 s | 0.11 s (all 14 rules then on by default) |
| lint, npm-groovy-lint's default rule set | 30.3 s | |
| format | did not finish in 15 min (two attempts) | 0.19 s |

The groovyfmt figures were measured on 2026-11-10 with a build between 0.5.0 and 0.6.0, when 14 rules were on by default. `groovyfmt --rules` lists the rules on today.

The comparison flatters groovyfmt somewhat. It skips the 9 spock files it can't parse, which npm-groovy-lint still processes, and CodeNarc's default rule set is far larger. All 9 are generated AST snapshots used as test fixtures, and 8 of them aren't valid Groovy source.
