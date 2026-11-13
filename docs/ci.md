# CI and pre-commit

Add `--strict` to `--check` or `--lint` to fail the job when a file was skipped (it doesn't parse, isn't UTF-8 or is over 10 MB). Otherwise such a file passes unchecked.

## GitHub Actions

The action installs the release binary for the runner (Linux or macOS, x64 or arm64, or Windows x64), checks it against its `.sha256`, puts it on `PATH` and runs it with `args`:

```yaml
- uses: actions/checkout@v5
- uses: costajohnt/groovyfmt@v0.9.2
  # with:
  #   args: --check --output-format github   # the default; "" only installs
  #   version: 0.9.2                         # default: the release of the action ref used
  #   token: ${{ github.token }}             # the default
- run: groovyfmt --lint --output-format github
```

`--output-format github` turns findings into annotations on the pull request diff.

Without `version`, the action installs the release named by the `Cargo.toml` at the ref used, so a release tag installs that release. A ref whose version has no release fails with a message naming it. Two such refs are `main` between a version bump and its release, and `v0.9.0`, whose release run failed. For the newest release, set `version: latest`.

On a Windows runner the action's steps run in bash (Git Bash, which GitHub's Windows images have) and unpack the `.zip` with PowerShell's `Expand-Archive`. Later steps find `groovyfmt` on `PATH` in `pwsh`, `cmd` or `bash`. This has not run on a GitHub Windows runner yet.

### Without the action

```yaml
- name: groovyfmt
  env:
    GH_TOKEN: ${{ github.token }}
  run: |
    gh release download --repo costajohnt/groovyfmt --pattern '*x86_64-unknown-linux-musl.tar.gz*'
    shasum -a 256 -c groovyfmt-*.tar.gz.sha256
    tar -xzf groovyfmt-*.tar.gz && mv groovyfmt-*/groovyfmt .
    ./groovyfmt --check --output-format github
    ./groovyfmt --lint --output-format github
```

### Code scanning

`--output-format sarif` uploads lint results to GitHub code scanning ([output formats](output-formats.md#sarif)).

## pre-commit

```yaml
repos:
  - repo: https://github.com/costajohnt/groovyfmt
    rev: v0.9.2
    hooks:
      - id: groovyfmt        # format in place; or groovyfmt-check to only report
      - id: groovyfmt-lint
```

These hooks run the `groovyfmt` on `PATH`. To have pre-commit build it from source instead (Rust 1.90+ and a C compiler, compiled once per `rev`), use `groovyfmt-source`, `groovyfmt-check-source` and `groovyfmt-lint-source`.

Or point a local hook at the binary on `PATH`:

```yaml
repos:
  - repo: local
    hooks:
      - id: groovyfmt
        name: groovyfmt
        entry: groovyfmt
        language: system
        files: (\.(groovy|gradle|gvy|gy|gsh)|(^|/)Jenkinsfile)$
```

pre-commit passes the staged files by name, and `exclude` in `.groovyfmt.toml` still applies to them. So generated code stays untouched without repeating the patterns in the hook.

## Gradle

The Gradle plugin (`gradle-plugin/`) adds three tasks:

| Task | Runs | Part of `check` |
|---|---|---|
| `groovyfmtCheck` | `groovyfmt --check` | yes |
| `groovyfmtLint` | `groovyfmt --lint` | yes |
| `groovyfmtFormat` | `groovyfmt` (formats in place) | no |

It downloads the release binary for the machine (Linux or macOS, x86_64 or arm64, or Windows x86_64) into `~/.gradle/groovyfmt/<version>/<target>/`, checks it against its `.sha256` before unpacking it, and reuses it after that. It needs Gradle 7.6 or later on Java 11 or later, and works with the configuration cache.

### Applying it from a checkout

The plugin is not on the Gradle Plugin Portal yet. Until it is, build it from a checkout of this repository with `includeBuild`:

```groovy
// settings.gradle
pluginManagement {
    includeBuild '../groovyfmt/gradle-plugin'  // path to the checkout
}
```

```groovy
// build.gradle
plugins {
    id 'base'  // or any plugin that adds check, such as java
    id 'io.github.costajohnt.groovyfmt'
}

groovyfmt {
    // version = '0.9.2'                          // default: the plugin's own version
    // binary = file('/usr/local/bin/groovyfmt')  // run this binary and download nothing
    // args = ['--strict']                        // added to every run
    // paths = ['src', 'build.gradle']            // default: the project directory
}
```

Once it is published, drop `includeBuild` and write `id 'io.github.costajohnt.groovyfmt' version '<version>'`.

### Behavior

- A task fails with groovyfmt's own output when files need formatting or lint finds something. It fails with a separate message when groovyfmt itself fails (exit code 2: bad arguments, a bad config, an unreadable file).
- The tasks are never up to date. groovyfmt finds its own files and configs, so Gradle can't tell when they changed.
- Apply the plugin to the root project only. groovyfmt searches the whole directory tree, subprojects included (it skips `build` directories), so one run covers a multi-project build. Applying it to subprojects too would check their files twice.

### Download and tokens

The download needs no token. To avoid GitHub's rate limit for anonymous downloads on shared CI machines, set one of these:

- the environment variable `GROOVYFMT_GITHUB_TOKEN` or `GITHUB_TOKEN`
- the Gradle property `groovyfmt.githubToken` in a `gradle.properties` file

The token is sent only to `api.github.com`, in a header. Don't pass it as `-P` on the command line, because Gradle 7.6 keeps every command-line `-P` value, in plain text, in its configuration cache.

With `--offline` the plugin uses the cached binary and fails if there is none. On Windows on ARM, which has no release binary, set `binary`.

### Without the plugin

With `groovyfmt` on `PATH`:

```groovy
tasks.register('groovyfmtCheck', Exec) {
    group = 'verification'
    commandLine 'groovyfmt', '--check'
}
tasks.register('groovyfmtLint', Exec) {
    group = 'verification'
    commandLine 'groovyfmt', '--lint'
}
check.dependsOn 'groovyfmtCheck', 'groovyfmtLint'
```
