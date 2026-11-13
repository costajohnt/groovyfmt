# Configuration

## The config file

Every key is optional:

```toml
indent = 4                  # spaces per block level, 1 to 16
continuation_indent = 8     # arguments, literals, continued lines (default: same as indent)
use_tabs = false            # true: indent with tabs, one per indent level
indent_after_labels = false # true: indent Spock when:/then: blocks under their label
normalize_spacing = false   # true: also fix the spaces between tokens within a line
max_line_length = 120       # wrap longer lines where that is safe, 40 to 400 (default: no wrapping)
exclude = ["generated/**", "**/*.gsh"] # skip these files and directories
baseline = ".groovyfmt-baseline.json"   # lint reports only what isn't in it

[lint]                      # rule = true/false; unlisted rules keep their default
UnnecessaryGString = true
UnusedImport = false
```

- `normalize_spacing` and `max_line_length` are described in [formatting](formatting.md).
- `baseline` and the `[lint]` rule names are described in [lint](lint.md).

Unknown keys and misspelled rule names are errors, so a typo can't silently do nothing. A config file can only fail the run when groovyfmt reaches a file or directory it governs. `-v` prints each config in use.

## Migrating from CodeNarc

`groovyfmt --import-codenarc` reads a CodeNarc configuration (ruleset XML, a Groovy ruleset, `codenarc.properties`, or npm-groovy-lint's `.groovylintrc.json`) and prints the `[lint]` table that carries it over. What it can't carry over follows as comments. It writes nothing, so redirect the output to start a config:

```console
$ groovyfmt --import-codenarc config/codenarc/codenarc.groovy > .groovyfmt.toml
```

Formatter settings that cover some of CodeNarc's formatting rules (`indent`, `normalize_spacing`) are suggested as comments above `[lint]`, since they change how files are formatted. [Lint](lint.md#migrating-from-codenarc) describes what it reads and reports.

## Which config applies

Each file is governed by the nearest `.groovyfmt.toml` or `groovyfmt.toml` found by walking up from that file's own directory, wherever groovyfmt runs from. With `--stdin-filepath`, the walk starts from that path's directory.

- `groovyfmt --check ../proj` uses `proj`'s config.
- One run from a repository root formats a subproject with its own config by that config.
- A nested config replaces its parent's completely. Nothing is merged, so repeat any settings the subproject should keep.
- The walk doesn't stop at the repository root. A config in a directory above the checkout (a shared workspace directory, a home directory) applies too. `-v` shows which one is used.
- Files with no config above them use the defaults.
- `--config <file>` skips discovery and applies that one file to everything.
- `--indent` overrides the indent of whichever config applies.
- `--rules`, and `--stdin` without `--stdin-filepath`, use the config for the current directory.

## exclude

`exclude` patterns are relative to the directory holding the config file, whatever directory groovyfmt runs from.

They match whole paths. `*` and `?` stay within one path segment, and `**` spans any number of segments. A matching directory is skipped with everything under it, including any config inside it. So `vendor` skips only the top-level `vendor`, `**/vendor` skips one at any depth, and `src/*.groovy` doesn't reach into `src/main/`.

A nested config's `exclude` applies only inside its own directory. Whether that directory is searched at all is up to the config above it. That holds for files named on the command line, `--stdin-filepath`, and runs from inside the excluded directory too. Pass `--config` to format such a directory anyway.

Excludes also apply to files named on the command line (pre-commit passes them that way) and to `--stdin-filepath`.

## Indentation

`continuation_indent = 8` matches IntelliJ's default and most Grails projects written in it. On OpenBoxes it cuts the first-run diff from 723 files to 453, and what remains is mostly trailing whitespace and runs of blank lines.

`use_tabs = true` writes leading indentation as tabs. Indents are worked out in columns as usual, with a tab `indent` columns wide, and each full tab width becomes a tab.

- With the default `continuation_indent` (the same as `indent`), a continued line gets one more tab than the line it continues.
- `continuation_indent = 8` with `indent = 4` gives it two.
- A `continuation_indent` that isn't a multiple of `indent` leaves the remainder as spaces after the tabs.

On spring-boot, which indents with tabs, `use_tabs = true` cuts the first-run diff from 762 files to 95. What remains is mostly missing final newlines, runs of blank lines and a few files that are indented with spaces.

Spock specs come in two styles. By default the statements of a `given:` / `when:` / `then:` block stay level with the label, as in Spock's documentation. `indent_after_labels = true` puts them one level deeper. In grails-core about half the specs use each style, so pick the one the project already uses before the first run.

## Finding the settings that fit

`groovyfmt --suggest-style [PATH]...` finds the settings that fit existing code before the first run. It formats every file in memory, writing nothing, under six styles:

- the defaults
- `indent = 2`
- `use_tabs = true`
- `continuation_indent = 8`
- `indent = 2` with `continuation_indent = 4`
- `use_tabs = true` with `continuation_indent = 8`

On stdout it prints the config lines for the style that leaves the most files unchanged, ready to paste into `.groovyfmt.toml` in place of any indent settings already there. On stderr it prints a table of how many files and lines each style would change.

A tie goes to the style that changes fewer lines, then to the defaults, then to the style with fewer settings. Everything else in the config applies as usual (`exclude`, `indent_after_labels`). `--indent` can't be combined with it, and it exits 2 when there are no files or none of them can be formatted. It ignores `max_line_length`.

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
