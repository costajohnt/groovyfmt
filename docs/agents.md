# Coding agents

A coding agent that edits a Jenkinsfile or a Gradle script has no compiler to tell it whether the file still reads as Groovy. groovyfmt answers in milliseconds, in a form an agent can act on: exit codes, `path:line:col` messages, and JSON.

## Instructions for the agent

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

## Claude Code hook

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

What it does:

1. It reads the edited path from the `tool_input.file_path` field of its JSON input, and does nothing for other files.
2. It formats the file in place, which changes nothing when the file doesn't parse.
3. It lints with `--strict`, so a file groovyfmt couldn't parse fails too.
4. When anything is found, the JSON report (lint violations, or `ParseError` diagnostics with line and column) goes to stderr and the hook exits 2. For PostToolUse, Claude Code then shows that stderr to Claude next to the tool result.

A clean file prints nothing to Claude. `exclude` in `.groovyfmt.toml` still applies, since the file is named on the command line.

## Language server

An agent or tool that speaks the Language Server Protocol can run `groovyfmt --lsp` instead of the CLI ([editors](editors.md#language-server)).

- It publishes parse errors and lint violations for each document it opens, with the same rules, config and positions as `--syntax` and `--lint`.
- It returns formatting as a text edit that changes only whitespace.
- It offers the `--lint --fix` fixes as code actions.
- It never writes a file. The edits go back to the client, which applies them.

## pre-commit

To check the same things before each commit, use the [pre-commit hooks](ci.md#pre-commit).
