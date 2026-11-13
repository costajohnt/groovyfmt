# Output formats

`--output-format` takes `text` (the default), `github`, `json` or `sarif`.

| Format | Works with | Output |
|---|---|---|
| `text` | every mode | file names or `path:line:col: message` lines, then a summary |
| `github` | every mode | GitHub Actions annotations on the pull request diff |
| `json` | `--check`, `--lint`, `--syntax`, formatting in place | one JSON document on stdout ([below](#json)) |
| `sarif` | `--lint`, `--syntax` | a SARIF 2.1.0 log for GitHub code scanning ([below](#sarif)) |

## github

`--output-format github` turns findings into annotations on the pull request diff, and turns each place a file could not be parsed into an annotation too. [CI](ci.md) shows it in a workflow.

## JSON

With `--output-format json`, stdout holds one JSON document and nothing else, one line per file.

- The summary line is left out, and so are the `-v` / `--strict` lists of skipped files, which the document already has.
- stderr still gets errors and `-v`'s `config:` lines.
- Exit statuses are unchanged. Exit 2 with an empty stdout is a usage or config error.

```console
$ groovyfmt --lint --output-format json
{"version":1,"files":[
{"path":"A.groovy","status":"unchanged","skip_reason":null,"diagnostics":[{"rule":"UnusedImport","severity":"warning","message":"The [a.b.Unused] import is never referenced","line":1,"column":1,"end_line":1,"end_column":18,"fixable":true}]},
{"path":"Broken.groovy","status":"skipped","skip_reason":"parse error","diagnostics":[{"rule":"ParseError","severity":"warning","message":"expected `}` to close the `{` on line 1","line":1,"column":10,"end_line":1,"end_column":10,"fixable":false}]}
],"summary":{"files":2,"unchanged":1,"would_reformat":0,"reformatted":0,"skipped":1,"errors":0,"diagnostics":2}}
```

The schema is versioned. `version` changes whenever a field is removed or changes meaning. New fields may be added within a version.

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

Strings are standard JSON. Every control character is escaped, and so is a `[` after `##` (as `[`). So file names and quoted source can't act as terminal escapes or GitHub Actions workflow commands when the document is printed in a log. A JSON parser reads back the original text.

Suppressed and baselined violations are left out ([lint](lint.md#suppressing-reports)).

## SARIF

`--output-format sarif`, with `--lint` or `--syntax`, prints a SARIF 2.1.0 log of the same diagnostics. Every rule (and `ParseError`) is described from the rule table. Upload it to GitHub code scanning:

```yaml
- run: groovyfmt --lint --output-format sarif > groovyfmt.sarif || true
- uses: github/codeql-action/upload-sarif@v4
  with:
    sarif_file: groovyfmt.sarif
```

Suppressed and baselined violations are left out. SARIF doesn't list baselined ones as suppressed results.
