---
type: project-note
status: active
updated: 2026-11-10
tags: [audit, code-review, groovyfmt, rust, formatter]
aliases: [groovyfmt audit, groovyfmt full audit]
---

# groovyfmt audit (2026-11-09, full repo)

Repo `/Users/johncosta/dev/groovyfmt` @ commit `169d448`, branch `main` (v0.4.0). Second audit of the day; the first is `docs/audit-2026-11-09.md` and its High findings were fixed before this one started, so this pass also attacks those fixes. Four dimensions (correctness, security, tests, architecture), run on Fable 5.2.

Process note: the four specialist subagents were dispatched but never returned a report and left no scratch work on the build host, so all four dimensions were done inline in the main session. Every Critical/High candidate, and most Mediums, was reproduced with a command on claude-worker against the release binary built from a clean `git archive 169d448` snapshot. That replaces the fresh-subagent verify pass. Anything not reproduced is marked (unverified).

Size: 3,225 tracked lines; Rust is 2,390 (`src/main.rs` 635, `src/lint.rs` 568, `src/format.rs` 518, `src/config.rs` 165, `src/lib.rs` 140, `tests/cli.rs` 364).

Baseline on claude-worker (cargo 1.98.1):

```
cargo fmt --check                                   exit 0, no output
cargo clippy --all-targets --locked -- -D warnings  Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.98s
cargo test --locked
  running 38 tests  test result: ok. 38 passed; 0 failed; 0 ignored
  running 3 tests   test result: ok. 3 passed; 0 failed; 0 ignored
  running 15 tests  test result: ok. 15 passed; 0 failed; 0 ignored
cargo build --release --locked                      Finished `release` profile [optimized] target(s) in 33.30s
```

56 tests, all green. The corpus gate (`groovyfmt --stats` over 23,766 files) is manual and not part of CI.

Not in scope: PR #1 (`--stdin-filepath`, config `exclude`), built in parallel, reviewed separately and merged as `64fcb10` while this audit ran. Line numbers below are for `169d448`.

What held up under attack: the core promise. 2,974 corpus files plus 2,866 copies with scrambled indentation, tabs, trailing whitespace, stray CRs and blank runs were formatted in place. 3,467 files changed, 0 had any non-whitespace byte change (checked independently of the tool's own token guard), 0 were non-idempotent at the default style or at `indent = 3, continuation_indent = 8, indent_after_labels = true`, and lint hit 0 internal errors. Eight 1 MB hostile parse inputs (`(`, `{`, `a.`, `"${`, `/*`, nested lists, ifs, ternaries) all finished in under 1.3 s. The earlier fixes for the symlink loop, `::` injection, lint stack overflow and indent blow-up were not broken.

---

## Critical

None.

---

## High

### H1. **The legacy `##[command]` workflow-command syntax passes through every output path**
`src/main.rs:316-338` (`printable`, `shown`) and `:481-499`. The earlier fix only stops a line from *starting* with `::`. The GitHub Actions runner also accepts the older `##[command]data` form and looks for it anywhere in the line: `ActionCommand.TryParse` does `message.IndexOf("##[")` (actions/runner `src/Runner.Common/ActionCommand.cs:132`), and `ActionCommandManager.cs:70-71` falls back to it whenever the `::` form doesn't match. Neither `printable` nor `shown` nor `gh_escape` touches `##[`, and lint messages quote source text. Reproduced: a file named `x##[error]boo.groovy` and a source line `def m = ['##[stop-commands]tok': 1, '##[stop-commands]tok': 2]` produce

```
M.groovy:1:37: DuplicateMapKey: Key `##[stop-commands]tok` appears more than once in this map
x##[error]boo.groovy:1:10: UnnecessarySemicolon: Semicolons as line endings can be removed safely
```

in text mode, the same text inside `::warning` lines in github mode, and `y##[add-mask]secret.groovy` from `--check`. A pull request can therefore forge error annotations, mask arbitrary strings in the log, or switch off command processing for the rest of the step (`stop-commands`) so later real annotations vanish. Not run against a live runner; the runner source is the evidence for the parsing side. Fix: neutralise `##[` at the same sinks that already call `printable` (for example print it as `#\u{200b}#[` or `##\[`), and add a test with `##[` in a file name and in a quoted key.

### H2. **Indent lookup re-walks siblings on every line: cubic on comment runs, quadratic on long blocks**
`src/format.rs:213-215`. For each line, `indent_for` calls `child.prev_sibling()` and keeps stepping back while the sibling is a comment. `prev_sibling()` in tree-sitter restarts from the parent's first child, so each call is O(siblings). A run of k comment lines that don't start at column 0 costs O(k³); any block of k statements costs O(k²). This is the same shape as the earlier label-scan finding, in the neighbouring loop. Reproduced with ` // c` lines (one leading space) inside `def f() { }` and at top level, and identically with `/* c */` lines:

```
  250 lines   0.05 s
  500 lines   0.33 s
 1000 lines   2.46 s
 2000 lines  19.4 s
 4000 lines  timeout 120 s (exit 124), a 24 KB file
```

Plain statements: 200,000 lines 4.3 s, 800,000 lines 32 s; a 5 MB file of ` x = 1` takes 25 s and 721 MB. `--lint` on the same inputs is linear (2000 comment lines 0.01 s). A commented-out block of a few hundred lines is already visible in an editor format-on-save, and one 24 KB file in a PR stalls `--check` in CI. Fix: stop asking for previous siblings per line. Walk each parent's children once with a cursor (lines are visited in order, so "kind of the last non-comment sibling before this child" can be carried forward or cached per parent the way `LabelCache` does), and add a timing regression test like the one PR #1 added for globs.

### H3. **The safety net that makes formatting "safe by construction" is not wired into any test**
`src/format.rs:59-62`. Deleting the token comparison (`|| tokens(src, &tree) != tokens(&out, &out_tree)`), or the whole post-format re-parse check (`if false {`), leaves all 56 tests green. `token_guard_sees_whitespace_inside_strings_and_comments` (`:385`) tests the `tokens()` helper in isolation; nothing asserts that `format()` returns `Skipped("token mismatch after formatting")`. A refactor could drop the guard and CI would not notice, and the README's central claim rests on it. Reproduced by mutation on a private copy:

```
MUTANT m8  token guard removed: SURVIVED (4 suites ok)
MUTANT m8b output re-parse check removed entirely: SURVIVED (4 suites ok)
```

Fix: make the guard testable from outside, for example split `format()` so a test can pass a deliberately wrong `reindent` result (or a test-only hook that corrupts one token) and assert `Skipped`; and put a reduced version of the corpus invariant check (format, strip whitespace, compare; format twice) into `cargo test` over a few dozen checked-in fixture files.

---

## Medium

- **M1.** `src/lint.rs:193-196`. UnusedImport (on by default) reports a class that is used only as `"$Name"` or `"$Name.CONST"` in a GString. The dollar interpolation is one leaf whose text includes the `$`, so `Name` never enters the `used` set. Reproduced: `import a.InDollar` with `println "$InDollar"` reports `The [a.InDollar] import is never referenced`, while `"${InStr.X}"` is fine. Deleting the import as advised breaks the code. Fix: for `gstring_dollar_interpolation` leaves, insert the identifier parts (strip `$`, split on `.`).
- **M2.** `src/main.rs:281-307`. The atomic rewrite creates a new inode, so the file takes the owner and group of whoever runs groovyfmt, hard links are split, and extended attributes and ACLs are dropped. Reproduced: a file owned by `agent:agent` formatted by root comes back `root:root`; with `ln H.groovy H2.groovy`, formatting `H.groovy` leaves `H2.groovy` unformatted and both at link count 1. Running a formatter as root in a container over a bind-mounted checkout is common, and afterwards the developer cannot edit their own files. Fix: after creating the temp file, `fchown` it to the original uid/gid where permitted, and fall back to an in-place write when the original has more than one link or ownership cannot be kept.
- **M3.** `src/main.rs:306` against the comment at `:171`. A symlink named on the command line is "followed" for reading but the rename then replaces the link itself. Reproduced: `ln -s real/A.groovy link.groovy; groovyfmt link.groovy` exits 0 saying `reformatted 1 of 1 files`, `link.groovy` is now a regular file and `real/A.groovy` is still unformatted. Fix: for top-level arguments resolve the path with `fs::canonicalize` before formatting, so the real file is rewritten and the link survives.
- **M4.** `src/main.rs:400-402`, `:417`. `par_map` returns every file's `Outcome::Formatted(String)` before the first one is used, so peak memory is the sum of all changed outputs plus each worker's parse tree, and `--check` keeps the full text only to count it. Reproduced: one 5 MB file peaks at 721 MB; 40 such files in `--check` took 297 s and peaked at 2,873 MB (`--lint`: 32 s, 1,546 MB). Highly repetitive files compress to almost nothing in git, so a small PR can do this to a CI runner. The time is H2; the memory is separate. Fix: in check mode return a bool instead of the text, in write mode write from the worker (or stream results in order), and consider a lower default size cap with a config override.
- **M5.** `src/main.rs:150-154`. The config is found from the working directory, not from the files being formatted (carried over from the first audit's M11; PR #1 fixed it for `--stdin-filepath` only). Reproduced: `proj/.groovyfmt.toml` has `indent = 2`; `groovyfmt --check` inside `proj` exits 0, `groovyfmt --check ../proj` from a sibling directory says `would reformat 1 of 1 files`, and in write mode would re-indent the project to 4. Same for a monorepo with a nested config. Fix: discover per top-level path argument (walk up from each), and group files by config.
- **M6.** `Cargo.toml:5`, `README.md:44`. `rust-version = "1.85"` and "Rust 1.85+" are wrong. Reproduced: `cargo +1.85.0 check --locked` fails with `rustc 1.85.0 is not supported by the following packages: tree-sitter-language@0.1.9 requires rustc 1.90`. The code also uses let-chains in `src/lint.rs`. Fix: set the real minimum and add a CI job that builds with it.
- **M7.** `.github/workflows/release.yml:19`. The Linux arm64 binary is dynamically linked against glibc: the v0.4.0 asset is `dynamically linked, interpreter /lib/ld-linux-aarch64.so.1` and needs `GLIBC_2.34`. It will not start on Alpine, Amazon Linux 2, RHEL 8, Debian 11 or Ubuntu 20.04, which is where arm64 CI runners and containers tend to be. The README lists it next to "x86_64 static" without saying so. Fix: build `aarch64-unknown-linux-musl` like the x86_64 target.
- **M8.** `.github/workflows/release.yml:3-5`. Any pushed `v*` tag builds and publishes a release without running the tests, without checking that the tag is on `main`, and without checking that it matches the version in `Cargo.toml`. Two of the four targets (`aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`) are never executed before upload, and they are built for the first time at tag time because CI doesn't build them. Fix: make the release job depend on the test job (`workflow_call` on ci.yml), compare `GITHUB_REF_NAME` with `cargo metadata`, and run the cross binaries under qemu or at least build them in CI.
- **M9.** Tests. Seven more one-line mutants survive all 56 tests, each undoing a fix from the first audit or a documented behaviour: the 10 MB size cap (`src/main.rs:264`), `gh_escape` on the lint message (`:489`) and on the file property (`:485`), the worker panic catch (`:233`), `printable` in `die()` (`:65`), the 0600 temp-file mode (`:301`), and the `modifier` half of the after-annotation rule (`src/format.rs:215`). Killed, for comparison: lint columns in characters, blank-file emptying, the `target` skip. Fix: one small test each; the escaping ones only need a file name containing `,`, `%` and a control character.

## Low (selected)

- `src/format.rs:243`: a line that starts with a map key or named argument called `else`, `catch` or `finally` loses its indent (`[\n    else: 1,` becomes `else: 1,` at column 0). Reproduced. Check the leaf's kind or its parent, not only its text.
- `src/main.rs:308-310`: when creating the temp file fails because the name exists, the cleanup deletes that pre-existing file. Reproduced: `.A.groovy.groovyfmt-<pid>` containing `precious` is gone after a run that exits 2 with `File exists`. Only remove the temp file if this call created it.
- `src/main.rs:294`: the temp name adds about 20 bytes, so a file whose name is longer than roughly 235 bytes can be read but not written (`File name too long (os error 36)`, exit 2). Reproduced.
- `src/main.rs:245` with `:404`: `h.join().unwrap_or_default()` drops a worker's whole result list if the thread dies outside `catch_unwind`, and the index is thrown away before `files.iter().zip(results)`, so later results would pair with the wrong paths in write mode (unverified; needs an allocation failure or similar). Keep the index and fail loudly on a short list.
- `src/main.rs:204-212`: overlapping arguments are processed once per mention (`--check A.groovy A.groovy .` lists `A.groovy` three times). Reproduced. Dedupe after the walk.
- `src/main.rs:94-141`: still open from the first audit. `--check=false` turns check on, `--` is an unknown option, and paths given with `--stdin` are silently ignored. Reproduced.
- `src/main.rs:451`, `:516`: files skipped for parse errors never affect the exit status, so CI is green while about 12% of a typical corpus is unchecked. Documented, but there is no `--strict`.
- `src/format.rs:92`, `:119`: tab indentation is always rewritten to spaces and there is no option for tabs; the README doesn't mention it. Reproduced.
- `src/lint.rs:408`: DuplicateCaseStatement compares raw text, so `case 'a':` and `case "a":` are not seen as the same label, although DuplicateMapKey normalises quotes. Reproduced.
- `src/lint.rs:223`: `comments.contains(symbol)` runs once per import over all comment text. 8,000 imports with 4.8 MB of comments take 2.95 s; the worst case inside the 10 MB cap is far higher (unverified beyond that point).
- Grammar: multi-line slashy and dollar-slashy strings (`def r = /a\n b\n/`) fail to parse, so the file is skipped. Reproduced.
- `src/config.rs:93`: discovery walks to the filesystem root, so a stray `/tmp/.groovyfmt.toml` or one in a parent of the checkout applies silently (unverified). `-v` shows which file was used; consider stopping at the repository root.
- `Cargo.toml:21`: the pinned grammar commit `25b1f4b` is reachable only as the head of the fork branch `groovyfmt-base`, with no tag. If that branch is rebuilt, a fresh clone can no longer fetch the rev. Tag each pinned rev on the fork.
- `.github/workflows/ci.yml:32`: `cargo build --release` in the smoke step lacks `--locked`.
- `.gitignore`: `.claude/` is not ignored, so agent worktrees show up as untracked.
- `src/lib.rs:8-13`: a new `Parser` per parse, two to four per file (carried over). A thread-local parser is a few lines.
- README: the documented GitHub Actions snippet downloads from this repository, which is private; another organisation's CI cannot use it without a token that can read it.

## Security summary

No memory-safety or file-system escape was found: there is no `unsafe` in the crate, symlinks below the named paths are still not followed, special files and oversized files are still skipped, and deep nesting is still handled. Two ways remain for a pull request to disrupt a CI job that runs groovyfmt on its content: log and annotation injection through the legacy `##[` command form (H1), and CPU exhaustion from a small file of indented comments (H2), with memory growth across many large files behind it (M4). On a developer machine the atomic write has side effects that matter more than the race it closed: ownership changes under root (M2) and a named symlink is replaced (M3). The workflows keep least-privilege permissions and SHA-pinned actions; the supply-chain gaps are that a tag publishes without tests (M8), checksums sit beside the binaries with no attestation (carried over), and the grammar pin depends on an untagged fork branch.

## Test quality

56 tests: 38 unit, 3 in the binary, 15 end-to-end. The end-to-end tests are well isolated (one temp directory per case) and the earlier "cannot fail" exit-code test is fixed. The weak spot is that fixes are tested at the helper level, not at the point of use. Gaps, in order:

1. The token guard and re-parse check can be deleted without a failing test (H3).
2. Nine of twelve mutants survive overall (H3, M9), including four of the first audit's security fixes.
3. No test covers time or memory, so H2 arrived unnoticed next to a fix for the same class of bug.
4. The corpus invariants (whitespace-only, idempotent) live outside the suite and outside CI. They held on 5,840 files in this audit, but only because someone ran them.
5. No test for symlinked arguments, hard links, ownership, duplicate arguments, or config discovery from a path argument (M2, M3, M5).
6. CI never builds the two cross targets or the declared minimum Rust version (M6, M8).

## Refactoring map

1. (S) One neutralising function for everything printed, covering control characters, leading `::` and `##[`, applied in `say`, `die` and the `eprintln!` sites; fixes H1 and makes the M9 escaping tests one table.
2. (S) Replace the per-line `prev_sibling` chain with a per-parent cache or a cursor carried across lines; fixes H2 and most of M4's time.
3. (S) Return `bool` from the check path and write from the worker in write mode; fixes M4's memory and removes the index/zip fragility.
4. (M) Split `write_atomic` policy: canonicalise named symlinks, keep owner, fall back to in-place for multi-link files, only delete temp files it created (M2, M3, two Lows).
5. (M) Per-path config discovery with files grouped by config (M5). PR #1 already made `Config` carry its root, which is most of the plumbing.
6. (S) Shared summary and skip bookkeeping for `fmt` and `lint_files` (carried over).

## Feature opportunities (ranked)

1. (S) Inline suppression (`// groovyfmt-ignore RuleName`) and a per-file off switch. There is no way to silence one lint hit today, and M1 shows false positives exist; without this a team's only option is turning the rule off.
2. (S) `--strict`: exit non-zero when files were skipped, so CI can't pass on files nobody checked.
3. (S) `--diff` in check mode. `Outcome::Formatted` already holds both texts.
4. (S) `use_tabs` style option; the indent engine already works in levels until the final `" ".repeat`.
5. (M) `--fix` for DuplicateImport, UnusedImport and UnnecessarySemicolon. The OpenBoxes PR removed 21 duplicate imports by hand; the token guard needs an allow-list of removed tokens.
6. (M) Distribution: musl arm64 (M7), a Windows build, a `.pre-commit-hooks.yaml`, and a public repo or mirrored release so other organisations' CI can download it.
7. (S) Run `--check` and `--lint` in one pass; they are separate processes that each walk and parse everything.
8. (M) `--output-format json` or SARIF for code-scanning upload.
9. (L) LSP with range formatting (carried over).

## Suggested first PR batch

1. Security and performance: H1, H2, M4, with the timing test and `##[` tests.
2. Test wiring: H3 and M9, plus a fixture-based invariant test in `cargo test`.
3. Write path: M2, M3 and the two temp-file Lows.
4. Lint and config correctness: M1, M5, the `else:` key Low.
5. Release hygiene: M6, M7, M8, grammar rev tag, `.gitignore`.
