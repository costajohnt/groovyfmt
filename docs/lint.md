# Lint

`groovyfmt --lint` has 67 rules, 54 of them on by default. `groovyfmt --rules` lists them with their state under the config that applies, and `groovyfmt --explain <Rule>` prints a rule's description, why it matters, flagged code and a fixed version, whether `--lint --fix` fixes it, and the `.groovyfmt.toml` lines to turn it off (or on).

The general rules are named after their [CodeNarc](https://codenarc.org/) equivalents, so existing configurations map across. The Jenkins, Gradle and Spock rules catch, in milliseconds, mistakes those tools otherwise report only when a build runs.

Files that don't parse are not linted, which keeps false positives out.

Turn a rule on or off in the `[lint]` table of [`.groovyfmt.toml`](configuration.md).

## Rules

### General

| Rule | Default | Flags |
|---|---|---|
| UnusedImport | on | an import whose name (or alias) never appears in code or as a word in a comment; not an annotated import (`@Library`, `@Grab`), which is there for its annotation. A static import of `getX`, `setX` or `isX` counts as used where the property `x` appears |
| DuplicateImport | on | the same import twice |
| UnnecessarySemicolon | on | a `;` at the end of a line, unless the next line starts with `{` (without the `;` that closure would become an argument of the statement before), `++` or `--` |
| EmptyCatchBlock | on | an empty `catch`, unless the parameter is named `ignore` or `ignored` |
| EmptyIfStatement | on | an `if` with an empty block |
| EmptyElseBlock | on | an `else` with an empty block |
| EmptyTryBlock | on | a `try` with an empty block |
| EmptyFinallyBlock | on | a `finally` with an empty block |
| EmptyWhileStatement | on | a `while` loop with an empty block; not one whose condition does the work with a call, an assignment or an increment, `while (in.read() != -1) {}` |
| EmptyForStatement | on | a `for` loop with an empty block; not a `for (;;)` loop whose condition calls, assigns or increments, as for `while` |
| DuplicateMapKey | on | a map literal with the same constant key twice (`a:` and `'a':` count as the same); the later entry wins |
| DuplicateCaseStatement | on | two `case` labels for the same constant (`'a'` and `"a"` count as the same) |
| AssignmentInConditional | on | `if (x = 1)`, where `==` was probably meant; `while (x = next())` is a common idiom and is exempt, and so is an assignment in extra parentheses, `if ((m = re.matcher(s)))` |
| ConstantIfExpression | on | an `if` whose condition is a literal, so one branch never runs |
| UnnecessaryGString | off | a double-quoted string with nothing to interpolate (noisy where double quotes are the house style) |
| UnusedSuppression | off | a `groovyfmt-ignore` comment that suppresses nothing (or names no rule), or a CodeNarc comment or `@SuppressWarnings` naming a groovyfmt rule it doesn't suppress ([suppressing reports](#suppressing-reports)) |
| ComparisonWithSelf | off | a variable compared with itself (Spock equality checks do this on purpose); not inside a GORM where query (`Book.where { title == title }`, `@Where`), where the left side is the domain property |

A block holding only a comment doesn't count as empty.

### Imports

| Rule | Default | Flags |
|---|---|---|
| UnnecessaryGroovyImport | on | an import from `java.lang`, `java.util`, `java.io`, `java.net`, `groovy.lang` or `groovy.util`, or of `java.math.BigDecimal` or `BigInteger`, which Groovy imports by default; not an aliased or static import, not in a file with another star import or a class of that name (either can be what the import is there to beat), and not in `*.gradle` files, where Gradle's own default imports (`org.gradle.api.tasks.Optional`) can need it |
| ImportFromSamePackage | on | an import of a class (or `.*`) from the file's own package |

### Likely bugs

Rules marked off found, on real code, almost only things written that way on purpose (mostly in tests). Turn them on where that doesn't apply.

| Rule | Default | Flags |
|---|---|---|
| DeadCode | on | a statement after `return`, `throw`, `break` or `continue` in the same block, on a later line; not a method or class declaration (a script's come after its `return`), a labeled statement, or a `break` right after a `return` or `throw` (the switch-case habit, kept on purpose so that removing the `return` can't let the case fall through); a second `break` right after a `break` serves no such purpose and is reported |
| EqualsAndHashCode | on | a class defining `equals(Object)` without `hashCode()`; not one with a superclass (which may define it), an annotation that generates it (`@EqualsAndHashCode`, `@Canonical`, `@Immutable`), an abstract class whose `equals(Object)` calls one of its abstract methods (the subclasses define equality, so `hashCode` belongs with them), or an `equals` that is never broader than identity (its body is just `false`, or `this.is(o)` and `o == null` tests joined by `\|\|`), which the inherited `hashCode()` already satisfies |
| EqualsOverloaded | on | an `equals` method with one parameter typed other than `Object`, which overloads `equals` instead of overriding it; not when the class, or a superclass declared in the same file, also declares `equals(Object)` (which can delegate to the overload), and not an abstract declaration |
| ObjectOverrideMisspelledMethodName | on | `equal(Object)`, `hashcode()`, `toSTring()` and other case misspellings of `equals`, `hashCode` and `toString` |
| ReturnFromFinallyBlock | on | a `return` in a `finally` block (not in a closure inside it) |
| EmptySwitchStatement | on | a `switch` with no cases |
| EmptyStaticInitializer | on | an empty `static { }` |
| EmptyInstanceInitializer | on | an empty `{ }` instance initializer |
| BrokenNullCheck | on | `x != null \|\| x.a` and `x == null && x.a`, which dereference `x` exactly when it is null |
| RemoveAllOnSelf | on | `c.removeAll(c)` instead of `c.clear()` |
| MissingNewInThrowStatement | on | `throw SomeException(...)` or `throw SomeException` without `new` (a capitalized name ending in `Exception`, `Failure` or `Fault`) |
| ExceptionNotThrown | on | a `catch` block ending in `new SomeException(...)` that is neither thrown nor the method's or closure's return value |
| ThrowExceptionFromFinallyBlock | off | a `throw` in a `finally` block, unless a `catch` inside the `finally` may handle it (rethrowing a saved exception after cleanup is a common idiom) |
| AssertWithinFinallyBlock | off | an `assert` in a `finally` block (tests check state after cleanup this way) |
| ToStringReturnsNull | off | a `toString()` that returns `null`, ends with `null`, or is empty (found only test fixtures doing it on purpose) |
| DuplicateSetValue | off | `[...] as Set` (or any type ending in `Set`) with the same constant twice (tests do it to check that a set drops them) |
| ConstantTernaryExpression | off | a ternary or elvis whose condition is a literal, `Boolean.TRUE` / `FALSE`, or a list or map literal |
| ConstantAssertExpression | off | an `assert` whose condition is a constant (`assert false` marks unreachable code in tests) |
| ComparisonOfTwoConstants | off | `==`, `!=`, `<`, `<=`, `>`, `>=`, `<=>`, `equals` or `compareTo` between two constants, or lists and maps of them (tests and `expect: 1 == 1` placeholders) |
| BrokenOddnessCheck | off | `x % 2 == 1`, false for negative odd numbers (nearly every real case is a count that is never negative) |

### Unused code

| Rule | Default | Flags |
|---|---|---|
| UnusedPrivateField | on | a private field whose name appears nowhere else in the file, nor in another file of the project ([below](#private-members-used-from-other-files)); not `serialVersionUID`, an annotated field (`@Delegate`, `@Autowired`; `@SuppressWarnings` and `@Deprecated` don't count), or a field of a class whose annotation reads fields (`includeFields`, `@Entity`, `@Builder`) |
| UnusedPrivateMethod | on | a private method whose name appears nowhere else in the file (for `getX`, `isX` and `setX`, nor the property `x`), nor in another file of the project ([below](#private-members-used-from-other-files)); not an annotated method, or one called by name: `methodMissing`, `readObject` and the like, Spock's `setup` / `cleanup` / `setupSpec` / `cleanupSpec`, GPars' `afterStart` / `afterStop` / `onException` / `onInterrupt` / `onTimeout` |
| UnusedVariable | on | a local variable whose name doesn't appear after its declaration in its block, closure or switch; not a script's top-level variable, `_`, `ignore` or `ignored`, Spock's `IOException e = thrown()` (there for its type), an annotated one (`@Field`), or a part of `def (a, b) = ...`. Assigning it counts as a use |

#### What counts as a use

A name counts as used wherever it appears outside comments, as a name or as a word in any string (`"$x"`, `getProperty('x')`, `@MethodSource('x')`). That can miss an unused one whose name is used elsewhere for something else, and it never reports a used one.

- A member reached through a computed name (`this."groupBy_${kind}"` or `"parse_$kind"()`) counts as used when its name starts with the literal part.
- `getProperty(name)`, `invokeMethod(name, args)` or `this[name]` with a computed name count as a use of every private member, as does any mention of `metaClass`.
- A declaration the grammar misreads, `final ClassNode name` without a value (read as a variable named `ClassNode`), is left alone.

#### Private members used from other files

Groovy doesn't enforce `private`. Another class can call a private method (`it.generateMethods()`, a `@Mixin` class's method called bare) or read a private field (`Constants.DEFAULT_PORT`).

So before UnusedPrivateField or UnusedPrivateMethod reports a member, the other Groovy files of its project are searched for its name in these forms:

- after `.`, `?.`, `*.`, `.&`, `.@` or `::` (also in a GString)
- as a whole string (`containsKey('name')`)
- for a method, called without a receiver (`name(` or `name {`)

Only a file that names the member's class counts (or its bean name, `fooService` for `FooService`). A file that declares a class of that name itself (a copy in another module) doesn't count.

The project is the nearest directory at or above the file holding `.git` or a config file, whatever files are named on the command line. So `--lint .`, `--lint src/A.groovy`, pre-commit, `--changed-since` and the LSP give the same answer.

- Its files are found as a directory walk finds them. Skipped and `exclude`d directories are left out and symlinks are not followed.
- They are read once per run, and only when some file has a report of these two rules. The LSP reads them again after a save.
- If the project has more than 20,000 Groovy files, more than 256 MiB of them, or one over 10 MiB, these two rules report nothing in it.

Matching by name means an unused member can be missed when another file of the project uses the same name for something else.

### Jenkinsfiles

These rules catch what the Declarative Pipeline plugin rejects when a build starts. They check a top-level `pipeline { }` in either of these:

- a file named `Jenkinsfile`, `Jenkinsfile.*` or `*.jenkinsfile`
- any file whose first statement it is (after comments and imports)

Scripted pipelines (`node { }`) are left alone. Directory walks find only files named `Jenkinsfile`, so name other pipeline files on the command line (with `--force` for a name other than `Jenkinsfile.*` or `*.jenkinsfile`).

The section lists follow the [syntax reference](https://www.jenkins.io/doc/book/pipeline/syntax/), and the plugin's own parser where they differ.

| Rule | Default | Flags |
|---|---|---|
| JenkinsMissingSection | on | a `pipeline` without `agent` or `stages`; `stages`, `parallel` or `matrix` without a stage; a stage with none of `steps`, `stages`, `parallel`, `matrix`; an empty `steps`, `post`, `when`, `environment`, `options`, `parameters`, `triggers` or `libraries` |
| JenkinsConflictingSections | on | a section twice in the same block, or a stage with more than one of `steps`, `stages`, `parallel`, `matrix` |
| JenkinsUnknownSection | on | a name that isn't a section where it stands (`steps` directly in `pipeline`, `parameters` in a stage, a misspelled `post` condition, the pre-0.8 `properties` or `notifications`), anything but `stage` in `stages`, and `stage`, `properties` or a `parallel` beside other steps in `steps` |
| JenkinsStatementOutsideSection | on | Groovy code (`def x = 1`, `if`, an expression) directly in `pipeline`, `stages`, a stage, `matrix`, `post` or `when` |
| JenkinsScriptRequired | on | `if`, `for`, `while`, `try`, `switch`, `def` or an assignment in `steps` (or in a block step inside it) without `script { }` |
| JenkinsUnknownWhenCondition | on | a `when` condition that isn't built in (`branch`, `expression`, `allOf` and so on) |
| JenkinsMisplacedOption | on | `parameters` or `triggers` inside `options`, or a pipeline-wide option (`buildDiscarder`, `disableConcurrentBuilds`, `skipStagesAfterUnstable`, ...) in a stage's `options` |
| JenkinsDuplicateStageName | on | two stages with the same name in one `stages` or `parallel` |
| JenkinsStageName | on | a stage with no name, or a name that isn't a plain string (`stage("Deploy ${env}")` is rejected) |
| JenkinsAgentForm | on | `agent` with no value, a bare `agent` other than `any` or `none`, or an `agent { }` block that is empty or names more than one agent type |
| JenkinsParallelNesting | on | `parallel` or `matrix` in a stage that is already inside a `parallel` or `matrix` (at any depth of `stages`), and `agent` or `tools` on the stage that holds a `parallel` or `matrix` |
| JenkinsWhenNesting | on | `allOf` or `anyOf` with no condition in its block, `not` with other than one, any of the three given arguments, and a block of conditions under any other condition (`branch(...) { }`); `expression { }` takes code and is fine |
| JenkinsMatrixAxes | on | a `matrix` without `axes`; an empty `axes`, `excludes` or `exclude`; an axis without `name` or `values`, with a name that isn't a plain string made of letters, digits and `_`, a value that isn't a plain string or number, or a name or value given twice; an exclude axis with both `values` and `notValues` |
| JenkinsInputSection | on | a stage's `input { }` without `message`, with a field other than `message`, `id`, `ok`, `submitter`, `submitterParameter` and `parameters`, or with a field twice or without exactly one value |
| JenkinsEnvironmentEntry | on | an `environment` line that isn't `KEY = value`, a key that isn't letters, digits and `_` (not starting with a digit) or is set twice, and a value that isn't a quoted string, a literal, a method call or literals joined with `+` (`FOO = params.BAR` is rejected; `FOO = "${params.BAR}"` is not) |
| JenkinsDuplicateOption | on | the same option twice in one `options`, or the same trigger twice in one `triggers` |
| JenkinsToolVersion | on | a `tools` line that isn't a tool type with exactly one installation name (`maven` alone, or two names) |

The new rules above follow the plugin's parser (`ModelParser.groovy`) and validator (`ModelValidatorImpl.groovy`) and their messages. On the plugin's own test pipelines, the Jenkins rules now catch 81 of the 132 invalid ones (49 before). The other 51 need the installed plugins or a running Jenkins to judge: step, option, trigger, parameter and agent parameters and types, tool installations, script security, and validator extensions from test plugins. Of the 289 other test pipelines, the rules report 6, each one a pipeline a test expects to fail or a test marked `@Ignore` (a matrix without `axes`).

Option names and agent types aren't checked against a list. Plugins add their own, and any block-scoped step is a valid option. Step parameters aren't checked either.

### Gradle build scripts

These apply in `*.gradle` files.

| Rule | Default | Flags |
|---|---|---|
| GradleRemovedConfiguration | on | `compile`, `runtime`, `testCompile` or `testRuntime` dependencies, removed in Gradle 7.0 (the message names the replacement); not when the build declares a configuration of that name itself, or when the nearest `gradle/wrapper/gradle-wrapper.properties` (in the script's directory or above) pins a Gradle below 7.0, which still has them |
| GradleJcenter | on | `jcenter()`, deprecated in Gradle 7.0 and removed in 9.0 |
| GradleLeftShiftTask | on | `task name << { }`, removed in Gradle 5.0 |
| GradleDynamicVersion | off | a `+` or `latest.*` dependency version (whether to pin is a policy choice) |
| GradleRemovedApi | on | what a Gradle upgrade guide lists as removed, with the version and the replacement in the message: the `maven` plugin and the `uploadArchives` task (7.0, use `maven-publish`); the `findbugs`, `jdepend`, `osgi`, `announce`, `build-announcements` and `compare-gradle-builds` plugins (6.0) and the old JVM and JavaScript plugins (7.0); `configurations.compile`, `.runtime`, `.testCompile` and `.testRuntime` (7.0, unless the build declares that configuration); and `archiveName`, `baseName`, `classifier`, `destinationDir`, `appendix` or `extension` set in an archive task's block (8.0, use `archiveFileName`, `archiveBaseName` and so on). An archive task's block is `jar`, `war`, `ear`, `bootJar`, `bootWar`, `shadowJar`, `sourcesJar`, `javadocJar`, `distZip` or `distTar` at the top level (or in `allprojects`, `subprojects`, `project`, `configure`, `afterEvaluate` or `tasks`), or a task block typed `Jar`, `Zip`, `Tar`, `War`, `Ear`, `ShadowJar`, `BootJar` or `BootWar`. Quiet when the nearest wrapper pins a Gradle older than the removal |

#### What the Gradle rules cover

The rules read the build script and the nearest wrapper version; they do not invoke Gradle or require the project to configure successfully.

Questions that need the resolved dependency graph are out of scope: unused dependencies, undeclared dependencies, duplicate classes across jars, and version recommendations all require a linting tool that runs inside the Gradle build. Those tools and groovyfmt cover different ground and work alongside each other.

These rules apply to Groovy DSL build scripts (`*.gradle`). Kotlin DSL scripts (`build.gradle.kts`, `settings.gradle.kts`) are not linted or formatted.

### Spock specifications

These apply in classes whose superclass is `Specification` or ends in `Spec` or `Specification` (a project's base spec). The fixture methods `setup`, `cleanup`, `setupSpec` and `cleanupSpec` are skipped, as Spock ignores labels there.

| Rule | Default | Flags |
|---|---|---|
| SpockBlockOrder | on | blocks in an order Spock refuses to compile (`then:` without `when:`, `when:` not followed by `then:`, `given:` after another block, anything but `and:`, `combined:` or `filter:` after `where:`), and labels that aren't Spock blocks |
| SpockDataTableColumns | on | a `where:` data table row with a different number of columns than the row above |
| SpockEmptyBlock | off | a `then:` or `expect:` block with no conditions, only a description (Spock accepts it, and Spock's own tests use empty blocks on purpose) |

## Fixing

`groovyfmt --lint --fix` removes, in place, what four rules flag:

- an `UnusedImport`, `DuplicateImport` or `ImportFromSamePackage` line, with its line break, and only when nothing but whitespace or a `;` shares the line
- an `UnnecessarySemicolon` `;`

An import from the file's own package is removed only when the file declares no class of the same name. An `UnnecessaryGroovyImport` is never removed, since it can be what beats a same-named class in the package, which one file can't show.

It then reports the violations that remain and exits 1 if there are any. Plain `--lint` marks fixable violations with ` (fixable)` (`"fixable": true` in JSON). A rule turned off in the config is neither fixed nor reported.

```console
$ groovyfmt --lint --fix
src/A.groovy:4:1: UnusedImport: The [b.C] import is never referenced
fixed 4 violations in 1 files (DuplicateImport 1, UnnecessarySemicolon 2, UnusedImport 1)
1 violations in 3 files (UnusedImport 1)
```

As with formatting, each fix is checked before the file is written:

- the fixed file must parse cleanly
- its tokens must be the original's minus exactly the removed ones
- its parse tree must keep the original's shape, so removing a `;` can't join two statements or turn one into another's body
- linting it must find the original's other violations and nothing new

A file that fails a check is left as it was and listed as `path not fixed: <why>` on stderr. A file that doesn't parse is never fixed. Writes go through the same path as formatting, so symlinks, permissions, owners, hard links and `exclude` are handled the same way. Running it again changes nothing.

`--lint --fix --diff` prints the fixes as a diff and writes nothing ([command line](usage.md#diffs)).

## Suppressing reports

A comment can turn rules off for part of a file. Without a rule list it covers every rule. Rule names are separated by commas or spaces, and `//` and `/* */` comments both work.

| Comment | Covers |
|---|---|
| `// groovyfmt-ignore [Rule, ...]` | its own line when it follows code there; the next line when it stands alone |
| `// groovyfmt-ignore-start [Rule, ...]` ... `// groovyfmt-ignore-end` | every line from the start comment to the end comment (or to the end of the file) |

CodeNarc's own forms work unchanged, so a project moving from CodeNarc keeps its suppressions:

| Form | Covers |
|---|---|
| `// codenarc-disable [Rule, ...]` ... `// codenarc-enable [Rule, ...]` | from the disable comment to an enable comment for the same rules (or the end of the file); a bare `codenarc-enable` ends them all |
| `// codenarc-disable-line [Rule, ...]` | its own line |
| `@SuppressWarnings('Rule')`, `@SuppressWarnings(['A', 'B'])` | the class, method, constructor, field or variable it annotates; on an import, the whole file. `'all'` and `'CodeNarc'` mean every rule |

A report is suppressed by the line it starts on. Suppressed reports are left out everywhere: the text, `github`, JSON and SARIF output, the exit status, `--lint --fix` (which doesn't fix them) and the language server.

The off-by-default rule `UnusedSuppression` reports a groovyfmt comment that suppresses nothing, and a CodeNarc comment or `@SuppressWarnings` naming a groovyfmt rule that it doesn't suppress. Other names, such as `unchecked` or CodeNarc-only rules, are left to their tools. It judges every rule, including ones turned off in the config.

Formatting has its own pair of comments, `// groovyfmt: off` and `// groovyfmt: on` ([formatting](formatting.md#turning-formatting-off)).

## Baseline

A baseline records the violations a repository has today, so the linter can be turned on for new code without fixing the old first.

```console
$ groovyfmt --lint --write-baseline
wrote 1204 violations in 312 files to .groovyfmt-baseline.json
```

With `baseline = ".groovyfmt-baseline.json"` in `.groovyfmt.toml`, or `--baseline FILE` on the command line, `--lint` then reports only violations that aren't in it, and exits 0 when those are all there is. The summary counts the ones the baseline held back and its stale entries (recorded violations that are gone):

```console
$ groovyfmt --lint
src/Build.groovy:12:5: DeadCode: This code can never run: ...
1 violations in 312 files (DeadCode 1), 1180 in the baseline, 24 stale baseline entries (--write-baseline drops them)
```

### Writing and updating it

`--write-baseline` writes to the first of these that is set:

1. `--write-baseline=FILE` (with `=`, since a separate word would be a path to lint)
2. the `--baseline` file
3. the config's `baseline`
4. `.groovyfmt-baseline.json` in the working directory

Run it again to drop stale entries. Entries for files the run didn't lint are kept.

- With `--strict`, stale entries fail the run (exit 1), so the baseline can only shrink. Fix a violation, then rewrite the baseline.
- `--lint --fix` leaves baselined violations alone (any violation of the same rule on a line the baseline recorded). Pass `--baseline ''` to ignore the baseline and fix them too.
- JSON and SARIF output leave baselined violations out, as they do suppressed ones. SARIF doesn't list them as suppressed results.

### The file

Each entry holds:

- the file's path relative to the baseline file's directory (with `/`)
- the rule
- a fingerprint of the text of the line the violation starts on, with whitespace runs collapsed so re-indenting doesn't matter
- its index among same-fingerprint violations of that rule in that file

Line numbers aren't recorded, so edits above a violation don't disturb it. Editing the line itself (beyond its spacing or its final `;`), or renaming or moving the file, makes its violations new.

The file is sorted JSON, the same on every machine, so it diffs cleanly in review:

```json
{"version":1,"entries":[
{"path":"src/Build.groovy","rule":"UnusedImport","fingerprint":"5f1c0d4a7b2e9c13","index":0}
]}
```

A baseline that isn't valid JSON, has another `version`, isn't a regular file or is over 10 MB is an error (exit 2), never silently ignored. The message names the file and the problem, never its content. Paths inside it are only compared as text, never opened.

### Trust

A config can come from an untrusted pull request. So its `baseline` must be a relative path inside the config's own directory, without `..` (anything else is a config error), and groovyfmt neither reads nor writes it through a symbolic link that leads out of that directory.

A file named on the command line (`--baseline FILE`, `--write-baseline=FILE`) is the user's own choice and can be anywhere.

The language server honors the config's baseline too. What it holds isn't published, and the server's fixes leave it alone.

### Adopting lint in an existing repository

1. `groovyfmt --rules` and turn off, in `.groovyfmt.toml`, any rule the project doesn't want.
2. `groovyfmt --lint --fix` to remove what can be removed safely.
3. `groovyfmt --lint --write-baseline` and commit `.groovyfmt-baseline.json`.
4. Add `baseline = ".groovyfmt-baseline.json"` to `.groovyfmt.toml`.
5. Run `groovyfmt --lint --strict` in CI, and rewrite the baseline whenever a fix leaves stale entries.
