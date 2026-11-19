# groovyfmt Gradle plugin

Formats and lints Groovy sources, Gradle build scripts and Jenkinsfiles in a Gradle build with [groovyfmt](https://github.com/costajohnt/groovyfmt), a fast formatter and linter written in Rust. The plugin downloads groovyfmt's release binary for the machine, checks it against its `.sha256`, and runs it. No JVM formatter and no extra dependencies.

## Usage

```groovy
// build.gradle
plugins {
    id 'base'  // or any plugin that adds check, such as java
    id 'io.github.costajohnt.groovyfmt' version '0.30.0'
}
```

```sh
./gradlew check            # fails when files need formatting or lint finds something
./gradlew groovyfmtFormat  # formats the files in place
```

Apply it to the root project only. groovyfmt walks the whole directory tree, subprojects included, so one run covers a multi-project build.

## Tasks

| Task | Runs | Part of `check` |
|---|---|---|
| `groovyfmtCheck` | `groovyfmt --check` | yes |
| `groovyfmtLint` | `groovyfmt --lint` | yes |
| `groovyfmtFormat` | `groovyfmt` (formats in place) | no |

A failing task prints groovyfmt's own output: the files that need formatting, or the lint findings with file, line and rule.

## Configuration

Everything is optional:

```groovy
groovyfmt {
    version = '0.30.0'                          // default: the plugin's own version
    binary = file('/usr/local/bin/groovyfmt')  // run this binary and download nothing
    args = ['--strict']                        // added to every run
    paths = ['src', 'build.gradle']            // default: the project directory
}
```

Formatting and lint rules are set in a `.groovyfmt.toml` file, the same one the command line and editor integrations read. See [configuration](https://github.com/costajohnt/groovyfmt/blob/main/docs/configuration.md) and [lint rules](https://github.com/costajohnt/groovyfmt/blob/main/docs/lint.md).

## Requirements

- Gradle 7.6 or later, on Java 11 or later
- Linux or macOS (x86_64 or arm64), or Windows x86_64
- Works with the configuration cache

The binary is cached in `~/.gradle/groovyfmt/<version>/<target>/`. Downloads need no token; on shared CI machines, set `GROOVYFMT_GITHUB_TOKEN` or `GITHUB_TOKEN` to avoid GitHub's anonymous rate limit.

## More

- [Gradle plugin details](https://github.com/costajohnt/groovyfmt/blob/main/docs/ci.md#gradle): behavior, tokens, using it from a checkout
- [groovyfmt](https://github.com/costajohnt/groovyfmt): the formatter, its rules and other integrations (GitHub Action, pre-commit, VS Code, JetBrains)
- [Issues](https://github.com/costajohnt/groovyfmt/issues)

License: [MIT](https://github.com/costajohnt/groovyfmt/blob/main/LICENSE)
