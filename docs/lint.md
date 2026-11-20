# Lint

`groovyfmt --lint` has 89 rules, 65 of them on by default. `groovyfmt --rules` lists them with their state under the config that applies, and `groovyfmt --explain <Rule>` prints a rule's description, why it matters, flagged code and a fixed version, whether `--lint --fix` fixes it, and the `.groovyfmt.toml` lines to turn it off (or on).

The general rules are named after their [CodeNarc](https://codenarc.org/) equivalents, so existing configurations map across. The Jenkins, Gradle and Spock rules catch, in milliseconds, mistakes those tools otherwise report only when a build runs.

Files that don't parse are not linted, which keeps false positives out.

Turn a rule on or off in the `[lint]` table of [`.groovyfmt.toml`](configuration.md). Coming from CodeNarc, `groovyfmt --import-codenarc` writes that table for you ([migrating from CodeNarc](#migrating-from-codenarc)).

## Rules

### General

| Rule | Default | Flags |
|---|---|---|
| UnusedImport | on | an import whose name (or alias) never appears in code (annotations on other imports included, `@Library('x') import ...`) or as a word in a comment; not an annotated import (`@Library`, `@Grab`), which is there for its annotation. A static import of `getX`, `setX` or `isX` counts as used where the property `x` appears |
| DuplicateImport | on | the same import twice |
| UnnecessarySemicolon | on | a `;` at the end of a line, unless the next line starts with `{` (without the `;` that closure would become an argument of the statement before), `++` or `--` |
| EmptyCatchBlock | on | an empty `catch`, unless the parameter is named `ignore` or `ignored` |
| EmptyIfStatement | on | an `if` with an empty block |
| EmptyElseBlock | on | an `else` with an empty block |
| EmptyTryBlock | on | a `try` with an empty block |
| EmptyFinallyBlock | on | a `finally` with an empty block |
| EmptyWhileStatement | on | a `while` loop with an empty block; not one whose condition does the work with a call, an assignment or an increment, `while (in.read() != -1) {}` |
| EmptyForStatement | on | a `for` loop with an empty block; not a `for (;;)` loop whose condition calls, assigns or increments, or whose update calls or reads a field (`n = n.next`; counting, `i += 1`, isn't work), as for `while`, and not a for-in loop over anything but a literal (`for (d in data) {}` may be draining an iterator) |
| DuplicateMapKey | on | a map literal with the same constant key twice (`a:` and `'a':` count as the same); the later entry wins |
| DuplicateCaseStatement | on | two `case` labels for the same constant (`'a'` and `"a"` count as the same) |
| AssignmentInConditional | on | `if (x = 1)`, where `==` was probably meant; `while (x = next())` is a common idiom and is exempt, and so is an assignment in extra parentheses, `if ((m = re.matcher(s)))` |
| ConstantIfExpression | on | an `if` whose condition is a literal, so one branch never runs |
| UnnecessaryGString | off | a double-quoted string with nothing to interpolate (noisy where double quotes are the house style). Fixable |
| UnusedSuppression | off | a `groovyfmt-ignore` comment that suppresses nothing (or names no rule), or a CodeNarc comment or `@SuppressWarnings` naming a groovyfmt rule it doesn't suppress ([suppressing reports](#suppressing-reports)) |
| ComparisonWithSelf | off | a variable compared with itself (Spock equality checks do this on purpose); not inside a GORM where query (`Book.where { title == title }`, `@Where`), where the left side is the domain property |
| DoubleNegative | off | `!!x` or `!(!x)` (`!!x` is a common way to turn a value into a boolean) |
| ForLoopShouldBeWhileLoop | off | `for (; condition;)`, a `for` loop with only a condition (a style preference) |
| ClassNameSameAsFilename | off | a file whose only top-level code is one class, interface, enum, trait or annotation type, named other than the file less its `.groovy` (a naming convention); a file with any other top-level code is a script and isn't checked |
| Groovy24EmptyElement | on | an empty argument, list element or map entry, `foo(a,,b)`, `[a, , b]`, `[a: 1,, b: 2]`: Groovy 2.4 (Hubitat, SmartThings) drops it, and Groovy 3 and later reject the file. It is on by default because the extra comma never does anything, so removing it is safe on every version. The formatter keeps it as written. Fixable |

A block holding only a comment doesn't count as empty.

### Imports

| Rule | Default | Flags |
|---|---|---|
| UnnecessaryGroovyImport | on | an import from `java.lang`, `java.util`, `java.io`, `java.net`, `groovy.lang` or `groovy.util`, or of `java.math.BigDecimal` or `BigInteger`, which Groovy imports by default; not an aliased or static import, not in a file with another star import or a class of that name (either can be what the import is there to beat), and not in `*.gradle` files, where Gradle's own default imports (`org.gradle.api.tasks.Optional`) can need it |
| ImportFromSamePackage | on | an import of a class (or `.*`) from the file's own package |

### Likely bugs

Rules marked off found, on real code, almost only things written that way on purpose (mostly in tests), or found nothing to measure them on. Turn them on where that doesn't apply.

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
| UnnecessaryInstanceOfCheck | on | `!x instanceof T`, which tests the boolean `!x` (`!` binds tighter), so it is always false, or always true for `Boolean`; `!(x instanceof T)` was meant |
| NegatedInCheck | on | `!x in xs` (or `!x !in xs`), which Groovy 2.4, 3 and 4 read as `(!x) in xs`, asking whether a boolean is in `xs`; `!(x in xs)` was meant. Not `!(x in xs)`, `(!x) in xs` or `x !in xs`. On the trial repos it found only real mistakes |
| UnintendedRegexPattern | on | `s.split('.')`, `s.split('\|')`, `s.replaceAll('.', r)` or `s.replaceFirst('\|', r)` with a quoted `.` or `\|`: these methods read a regular expression, where `.` matches any character and `\|` the empty string, so `'a.b'.split('.')` is empty. Not a slashy string, a `split` on `Pattern.compile(...)` or Guava's `Splitter.on(...)` (which take the text), a replacement that isn't a plain string (a closure, a variable), or `replaceAll('.', c)` with one character `c` other than a separator (` `, `=`, `*`, `X`), which builds a run as long as the text on purpose. A separator, one of `_ - / : , ; \|`, still counts: `replaceAll('.', '-')` is far more often a dot meant to become a dash than a Markdown underline |
| ContradictoryComparison | on | `x != a \|\| x != b` (always true) or `x == a && x == b` (never true) for two different constants (strings, integers, booleans, null) and the same name or property chain `x`, anywhere in one chain of `\|\|` or `&&`. Not decimals (`1 == 1.0` in Groovy), a subject with a call in it, or a chain that assigns or increments something |
| ImpossibleSizeCheck | on | `x.size() >= 0` (always true) or `x.size() < 0` (never true), also `length()` and 0 on the left, in the condition of an `if`, `while`, `do ... while`, `for`, ternary or elvis, through `!`, `&&` and `\|\|`. Not where it changes nothing (an always true part of `&&`, a never true one of `\|\|`, counting each `!`), and not in an assertion or a Spock condition: on the corpus, 45 of 46 there were tests checking on purpose only that a call returns |
| CaseConvertedComparison | on | `s.toLowerCase() == 'Text'` with an uppercase ASCII letter in the string, or `s.toUpperCase()` with a lowercase one, by `==`, `!=` or `equals`: always false (or true for `!=`) |
| ThrowExceptionFromFinallyBlock | off | a `throw` in a `finally` block, unless a `catch` inside the `finally` may handle it (rethrowing a saved exception after cleanup is a common idiom) |
| AssertWithinFinallyBlock | off | an `assert` in a `finally` block (tests check state after cleanup this way) |
| ToStringReturnsNull | off | a `toString()` that returns `null`, ends with `null`, or is empty (found only test fixtures doing it on purpose) |
| DuplicateSetValue | off | `[...] as Set` (or any type ending in `Set`) with the same constant twice (tests do it to check that a set drops them) |
| ConstantTernaryExpression | off | a ternary or elvis whose condition is a literal, `Boolean.TRUE` / `FALSE`, or a list or map literal |
| ConstantAssertExpression | off | an `assert` whose condition is a constant (`assert false` marks unreachable code in tests) |
| ComparisonOfTwoConstants | off | `==`, `!=`, `<`, `<=`, `>`, `>=`, `<=>`, `equals` or `compareTo` between two constants, or lists and maps of them (tests and `expect: 1 == 1` placeholders) |
| BrokenOddnessCheck | off | `x % 2 == 1`, false for negative odd numbers (nearly every real case is a count that is never negative) |
| BigDecimalInstantiation | off | `new BigDecimal(1.1)` or `new java.math.BigDecimal(1.1d)`: a BigDecimal made from a decimal or double literal, which goes through an inexact double; not `1.1f` or a whole number, as in CodeNarc (found only in tests of BigDecimal itself, with values a double holds exactly) |
| BitwiseOperatorInConditional | off | `\|` or `&` in the condition of an `if`, `while`, ternary or elvis, through the binary operators the condition is made of (`(flags & MASK) != 0` too, as in CodeNarc); not inside a call or a `!` (nearly every real one tests bit flags on purpose) |
| EmptySynchronizedStatement | off | a `synchronized` block with nothing in it; a comment counts as content, as for the other empty blocks (the projects below gave nothing to measure it on) |
| IntegerGetInteger | off | `Integer.getInteger(name)` or `Integer.getInteger(name, default)`, which reads a system property rather than parsing `name` (found mostly deliberate reads of system properties) |
| RandomDoubleCoercedToZero | off | `Math.random()` cast to `int`, `Integer`, `long` or `Long`, the initial value of a field of one of those types, or returned (directly or from a ternary or elvis branch) from a method declared to return one; not a local variable, as in CodeNarc, nor a `return` inside a closure in the method (nothing to measure it on) |
| StringLeftShiftDiscarded | on | `s << x` as a statement on a local String: Groovy's `String.leftShift` returns a new StringBuffer and leaves `s` unchanged, so whatever it meant to collect is lost (checked with Groovy 4.0.24). Only for a local declared `String` or with a plain string literal (`def s = ''`) and never assigned anything but a plain string literal, and only where the statement's value goes nowhere: not the last statement of a closure (but one passed to `each`, `eachWithIndex`, `reverseEach`, `forEach` or `times`) of a method that isn't `void`, or a script's last statement. The message offers a StringBuilder or `s += x` without naming either as the fix: both change what later code reading `s` gets |
| UnnecessaryModOne | off | `x % 1`, always 0 for a whole number (on real code, `x % 1 != 0` tested decimals for a fractional part, which works) |

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

A method at the top level of a `.gradle` script is also a method of its project, which another build script can call as `rootProject.f()`, `project(':a').f()`, `parent.f()`, or bare from a subproject. So it counts as used when any other `.gradle` script of the project has its name anywhere outside comments and prose strings.

The project is the nearest directory at or above the file holding `.git` or a config file, whatever files are named on the command line; with neither, the nearest holding `settings.gradle` or `settings.gradle.kts`. So `--lint .`, `--lint src/A.groovy`, pre-commit, `--changed-since` and the LSP give the same answer.

- Its files are found as a directory walk finds them. Skipped and `exclude`d directories are left out and symlinks are not followed.
- They are read once per run, and only when some file has a report of these two rules. The LSP reads them again after a save.
- If the project has more than 20,000 Groovy files, more than 256 MiB of them, or one over 10 MiB, these two rules report nothing in it.

Matching by name means an unused member can be missed when another file of the project uses the same name for something else.

A private member used only by a DSL script evaluated with the class as its delegate (a `DelegatingScript`, a builder) is reported, since nothing in Groovy code names it: the script calls it bare. Removing it breaks the script, so mark it `@SuppressWarnings('UnusedPrivateMethod')` or `@SuppressWarnings('UnusedPrivateField')`, or put that on the class to cover all its members. `--explain` says the same.

### Jenkinsfiles

These rules catch what the Declarative Pipeline plugin rejects when a build starts. They check a top-level `pipeline { }` in either of these:

- a file named `Jenkinsfile`, `Jenkinsfile.*`, `Jenkinsfile_*`, `Jenkinsfile-*`, `*-Jenkinsfile`, `*_Jenkinsfile`, `*.jenkinsfile` or `*.jenkins`
- any file whose first statement it is (after comments and imports)

They also check the pipelines of a shared library's steps: a `.groovy` file directly in a `vars` directory, where a `pipeline { }` in a top-level `call` method is checked, whether it stands in the method's body (after any other statements) or in a branch of an `if` there that picks one of several pipelines. Reports point at the lines in the file. A `pipeline { }` in a closure, a loop or a `try` inside `call`, or in a method with another name, is left alone.

```groovy
// vars/buildApp.groovy
def call(Map config) {
    config.label = config.label ?: 'linux'
    pipeline {
        agent { label config.label }
        stages { ... }
    }
}
```

Scripted pipelines (`node { }`) are left alone. Directory walks find pipelines named `Jenkinsfile`, `Jenkinsfile.*`, `Jenkinsfile_*`, `Jenkinsfile-*`, `*-Jenkinsfile`, `*_Jenkinsfile`, `*.jenkinsfile` or `*.jenkins` ([which files](usage.md#which-files)); name other pipeline files on the command line, with `--force`. A script found by its `#!groovy` line is Groovy, not a pipeline by its name: the rules check it only when `pipeline { }` is its first statement.

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
| JenkinsEnvironmentEntry | on | an `environment` line that isn't `KEY = value` (a declaration of one variable with a value, `def KEY = 'v'`, counts as one, as Declarative reads it), a key that isn't letters, digits and `_` (not starting with a digit) or is set twice, and a value that isn't a quoted string, a literal, a method call or literals joined with `+` (`FOO = params.BAR` is rejected; `FOO = "${params.BAR}"` is not) |
| JenkinsDuplicateOption | on | the same option twice in one `options`, or the same trigger twice in one `triggers` |
| JenkinsToolVersion | on | a `tools` line that isn't a tool type with exactly one installation name (`maven` alone, or two names) |
| JenkinsEnvBooleanComparison | on | `env.NAME == true` or `env.NAME != false` (either boolean, either side) in any file, scripted pipelines and library code included: Jenkins keeps environment variables as strings, so `==` is always false and `!=` always true. Not in a file that declares its own `env`: a variable, field, method, closure or catch parameter, loop variable, or a script binding `env = ...` |

The new rules above follow the plugin's parser (`ModelParser.groovy`) and validator (`ModelValidatorImpl.groovy`) and their messages. On the plugin's own test pipelines, the Jenkins rules now catch 81 of the 132 invalid ones (49 before). The other 51 need the installed plugins or a running Jenkins to judge: step, option, trigger, parameter and agent parameters and types, tool installations, script security, and validator extensions from test plugins. Of the 289 other test pipelines, the rules report 6, each one a pipeline a test expects to fail or a test marked `@Ignore` (a matrix without `axes`).

Option names and agent types aren't checked against a list. Plugins add their own, and any block-scoped step is a valid option. Step parameters aren't checked either.

### Gradle build scripts

These apply in `*.gradle` files.

| Rule | Default | Flags |
|---|---|---|
| GradleRemovedConfiguration | on | `compile`, `runtime`, `testCompile` or `testRuntime` dependencies, removed in Gradle 7.0 (the message names the replacement), in the project's `dependencies { }`: at the top level or in `allprojects`, `subprojects`, `project`, `configure` or `afterEvaluate` or a plugin callback (`plugins.withType(JavaPlugin) { }`, `pluginManager.withPlugin('java') { }`, `plugins.withId('java') { }`), not one inside another block, such as NeoGradle's `runs { configureEach { dependencies { runtime x } } }`, whose only configuration is `runtime`; not when the build declares a configuration of that name itself (`configurations { compile }`, `configurations.create('compile')`), or a `.gradle` script in the script's directory or one above it (up to the build's root) declares it in `allprojects { }` or `subprojects { }` or applies the script (`apply from:`) and declares it, or when the nearest `gradle/wrapper/gradle-wrapper.properties` (in the script's directory or above) pins a Gradle below 7.0, which still has them, or when nothing shows a Java plugin may be there: neither the script nor another `.gradle` script in its directory or one above it (up to the build's root) applies one (or a Grails plugin, which applies Groovy's) or a script (`apply from:`, which may), and the script declares no dependency in a configuration they make (`implementation`, `api`, `testImplementation`, ...). Then a plugin from elsewhere made the configuration, as gradle-hdvl's SystemVerilog plugin makes `compile` and `testCompile`. Fixable ([fixing](#fixing)) |
| GradleJcenter | on | `jcenter()`, deprecated in Gradle 7.0 and removed in 9.0. Fixable |
| GradleLeftShiftTask | on | `task name << { }`, removed in Gradle 5.0; quiet when the nearest `gradle/wrapper/gradle-wrapper.properties` (in the script's directory or above) pins a Gradle below 5.0, and reported when there is no wrapper. Fixable |
| GradleSpaceAssignment | on | Groovy DSL space assignment, `name value` for `name = value`, which Gradle 8.12 deprecated and Gradle 10 removes: `description`, `group` or `version` setting the project's property (at the top level, or in `allprojects`, `subprojects` or `project(':x')`), or `description` or `group` in a task's block (`task x { }`, `task x(type: T) { }`, `task('x') { }`, `tasks.register`, `tasks.create` or `tasks.named`), as a statement of its own: not `group 'a' version 'b'` or `version 'a' call()`, which Groovy reads as a call chain (`group('a').version('b')`), nor a line a `\` continues. Quiet when the nearest wrapper pins a Gradle before 8.12, and in `settings.gradle` and init scripts. Fixable |
| GradleDependencyTuple | off | a dependency in map notation, `implementation group: 'a', name: 'b', version: '1'`, where the string `'a:b:1'` says the same: only `group`, `name` and `version` keys (`group` and `name` required), each a plain string. Gradle 9.1 deprecated this notation; off by default as a style choice for builds on older Gradle. Fixable |
| GradleDependencyParentheses | off | a dependency in parentheses with no closure after it, `implementation('a:b:1')`, whose arguments (strings, names, dotted names, calls without a closure, named arguments, on one line) read the same without them (a style choice). Fixable |
| GradleDynamicVersion | off | a `+` or `latest.*` dependency version (whether to pin is a policy choice) |
| GradleRemovedApi | on | what a Gradle upgrade guide lists as removed, with the version and the replacement in the message: the `maven` plugin and the `uploadArchives` task (7.0, use `maven-publish`); the `findbugs`, `jdepend`, `osgi`, `announce`, `build-announcements` and `compare-gradle-builds` plugins (6.0) and the old JVM and JavaScript plugins (7.0); `configurations.compile`, `.runtime`, `.testCompile` and `.testRuntime` (7.0, unless the build declares that configuration, as in the cases GradleRemovedConfiguration lists, or an `if` or `?:` around it checks for it with `configurations.hasProperty('compile')` or `findByName`); and `archiveName`, `baseName`, `classifier`, `destinationDir`, `appendix` or `extension` set in an archive task's block (8.0, use `archiveFileName`, `archiveBaseName` and so on); and, under a wrapper pinning Gradle 9 or later, setting a project property the plugin conventions made (9.0): `sourceCompatibility`, `targetCompatibility`, `docsDirName`, `testResultsDirName`, `testReportDirName` (use `java { }`), `archivesBaseName`, `distsDirName`, `libsDirName` (use `base { }`), `mainClassName` (use `application { mainClass = ... }`), `applicationName`, `applicationDefaultJvmArgs`, `executableDir` (use `application { }`), `webAppDirName`, `appDirName`, `libDirName` and `projectReportDirName`, at the top level or in `allprojects`, `subprojects`, `project`, `configure` with an argument naming a project (`configure(subprojects)`, not `configure([tasks.compileJava])`) or `afterEvaluate` (and `mainClassName` in `application { }`, `archivesBaseName` in `base { }`; `archivesBaseName`, `distsDirName` or `libsDirName` in an archive task's block, `jar { }` or `tasks.jar { }`; any but `sourceCompatibility` and `targetCompatibility` in `java { }`; and any in a plugin callback, `pluginManager.withPlugin('java') { }`, `plugins.withId` or `plugins.withType`, all of which fall through to the project), unless the script makes it an extra property or a `gradle.properties` up to the wrapper's directory sets it; reading one too, or the read-only `docsDir`, `testResultsDir`, `testReportDir`, `applicationDistribution`, `webAppDir`, `deploymentDescriptor` and `projectReportDir` (9.0), or `libsDir` and `distsDir` (7.0, under no wrapper or one pinning 7 or later; use `base.libsDirectory`): as `project.libsDir` (also `"${project.libsDir}"`) outside a class, or bare at the top level, in `allprojects`, `subprojects`, `project`, `afterEvaluate` or a `withPlugin` or `withId` callback, or in an untyped task's block (`task x { }`, `tasks.register('x') { }`) or its `doLast { }` or `doFirst { }`, but not in a script method, a typed task's block or a closure with a parameter of that name, and not when the script or a `.gradle` script in a directory above it makes it an extra property, the script tests for it with `hasProperty('mainClassName')` (an extra property its subprojects may set), or a `gradle.properties` sets it; also under such a wrapper, `exec { }` and `javaexec { }` on the project or a build, settings or init script (9.0: bare, `exec({ })`, `project.exec { }` or `rootProject.exec { }`, anywhere but in a class or a script with its own `exec` method; inject ExecOperations, or use `providers.exec`) and `main` set in a JavaExec task's block (9.0, use `mainClass`), a task block typed `JavaExec` or `BootRun`, or `bootRun`; a report's `destination` set in a `reports { }` block or on `reports.` (`html.destination = f`, `xml.destination f`, `reports { junitXml { destination = f } }`; 9.0, set `outputLocation`; a read is left alone, as it fails on 8.x too); a copy spec's `fileMode` or `dirMode` in a Copy, Sync or archive task's block (also `processResources`, `processTestResources`, `installDist`), `copy { }`, `sync { }` or a child `from(x) { }` or `into(x) { }` in one of those or a distribution's `contents { }`, not a plugin's own spec such as nebula's `ospackage { }` (9.0, use `filePermissions { unix(...) }` and `dirPermissions { unix(...) }`, Gradle 8.3 and later); and the IDEA module's `testSourceDirs` and `testResourceDirs` in `idea { module { } }` or as `idea.module.testSourceDirs` (9.0, use `testSources.from(...)` and `testResources.from(...)`). Also under such a wrapper, in any `.gradle` script: `XmlSlurper`, `XmlParser` and `FileNameFinder` used bare, which found `groovy.util`'s through Groovy's default imports, unless the script imports that name (`import groovy.xml.XmlSlurper`, `import a.XmlSlurper`, `as XmlSlurper`) or the new package with `.*`, or declares a class of that name; and `groovy.util.XmlSlurper`, `groovy.util.XmlParser`, `groovy.util.FileNameFinder` and `groovy.util.slurpersupport` in an import or a qualified name. Gradle 9.0 runs scripts with Groovy 4, which moved them to `groovy.xml` (`groovy.ant` for `FileNameFinder`), so the script fails with "unable to resolve class XmlSlurper"; the new names are in Groovy 3, so Gradle 8 has them too. Also under such a wrapper, an import (static too) of an `org.gradle.util` class Gradle 9.0 removed, which fails the same way: `ClosureBackedAction`, `CollectionUtils`, `ConfigureUtil`, `DistributionLocator`, `GFileUtils`, `GUtil`, `NameMatcher`, `NameValidator`, `RelativePathUtil`, `SingleMessageLogger`, `TextUtil`, `VersionNumber` and `WrapUtil` (`GradleVersion`, `Path` and `Configurable` remain). An archive task's block is `jar`, `war`, `ear`, `bootJar`, `bootWar`, `shadowJar`, `sourcesJar`, `javadocJar`, `distZip` or `distTar` at the top level (or in `allprojects`, `subprojects`, `project`, `configure`, `afterEvaluate` or `tasks`), or a task block typed `Jar`, `Zip`, `Tar`, `War`, `Ear`, `ShadowJar`, `BootJar` or `BootWar`. Quiet when the nearest wrapper pins a Gradle older than the removal |

#### What the Gradle rules cover

The rules read the build script and the nearest wrapper version; they do not invoke Gradle or require the project to configure successfully.

GradleSpaceAssignment reports only properties known to be properties where the script's delegate is known. Gradle deprecated space assignment for every property, but a name can also be a real method (`id 'x' version '1'` in `plugins { }`, a plugin extension's own `name(String)`), which one file can't tell apart, so `description 'x'` in a `jar { }` block, `version '1'` in a publication and any other property are not reported. On the Groovy build scripts of 8 field-trial checkouts (6,171 `*.gradle` files), it reported 322 places; a fixed-seed sample of 40 had no false positives. The two dependency rules apply to the calls in a `dependencies { }` block other than `add`, `project`, `platform` and the handler's other methods; their samples of 40 (of 456 and 10,372 reports) had none either.

Unused dependencies, undeclared dependencies, duplicate classes across jars and version recommendations all need the resolved dependency graph, so they are out of scope and require a linting tool that runs inside the Gradle build. Those tools and groovyfmt cover different ground and work alongside each other.

These rules apply to Groovy DSL build scripts (`*.gradle`). Kotlin DSL scripts (`build.gradle.kts`, `settings.gradle.kts`) are not linted or formatted.

### Spock specifications

These apply in classes whose superclass is `Specification` or ends in `Spec` or `Specification` (a project's base spec), and in a class with block labels whose superclass chain, through its own file and then the other files of its project (the nearest directory holding `.git` or a config file), reaches one. A class whose chain ends at a class extending nothing is not a spec. When neither can tell (a base from a library, or no project), a class with block labels is taken for a spec only in a file that imports from `spock.`. The fixture methods `setup`, `cleanup`, `setupSpec` and `cleanupSpec` are skipped, as Spock ignores labels there.

| Rule | Default | Flags |
|---|---|---|
| SpockBlockOrder | on | blocks in an order Spock refuses to compile (`then:` without `when:`, `when:` not followed by `then:`, `given:` after another block, anything but `and:`, `combined:` or `filter:` after `where:`), and labels that aren't Spock blocks |
| SpockDataTableColumns | on | a `where:` data table row with a different number of columns than the row above |
| SpockEmptyBlock | off | a `then:` or `expect:` block with no conditions, only a description (Spock accepts it, and Spock's own tests use empty blocks on purpose) |
| SpockUnassertedCondition | on | a comparison (`==`, `<`, `instanceof`, `in`), a predicate call on a receiver (`contains`, `containsAll`, `containsKey`, `containsValue`, `endsWith`, `equals`, `equalsIgnoreCase`, `hasErrors`, `hasNext`, `isAssignableFrom`, `isClosed`, `isEmpty`, `isPresent`, `matches`, `startsWith`, not in an `assertThat(...)` chain), a property read on a plain path (`x.done`, `it?.valid`, `snapshots[0].terminal`, `this.ok`: names, properties and indexes that are literals or names, no call anywhere, not inside a `try`, which may be waiting for the read to throw), and `!`, `&&`, `||` over them, in a `then:` or `expect:` block nested in `if`/`else`, a loop, a `switch` case, `try`, `synchronized` or a closure passed to `each`, `eachWithIndex`, `reverseEach`, `forEach` or `times` (also in a `with { }` or `verifyAll { }`, and through Groovy's `it.with { }` in one); a predicate may be called without parentheses (`it.startsWith "y"`). Spock asserts only top-level statements, so the check never runs; write `assert`. Not reported inside `with { }` (Spock's `with(x) { }` or, at the top level, Groovy's `x.with { }`), `verifyAll { }`, `verifyEach`, `interaction { }`, argument constraints or `@Verify` / `@VerifyAll` helpers, which Spock asserts, or in a closure whose value its method returns (`every { }`, `find { }`) |

## Fixing

`groovyfmt --lint --fix` fixes, in place, what these rules flag:

- an `UnusedImport`, `DuplicateImport` or `ImportFromSamePackage` line is removed, with its line break, and only when nothing but whitespace or a `;` shares the line
- an `UnnecessarySemicolon` `;` is removed
- `Groovy24EmptyElement`: the extra comma is removed, with the whitespace back to the comma before it, `foo(a,,b)` becoming `foo(a,b)` (only the comma when a comment is between them). Groovy 2.4 drops the empty element, so the call, list or map is the one 2.4 already compiled, and Groovy 3 and later now accept it. It is fixed only when it is the only empty element in its brackets and no trailing comma follows: Groovy 2.4 rejects `foo(a,,,b)`, `foo(a,,b,,c)` and `[1,,2,]` as well, so code like that compiled nowhere, and it is only reported
- `UnnecessaryGString` (when turned on): `"text"` becomes `'text'` and `"""text"""` becomes `'''text'''`, the content kept as written. Groovy compiles both to the same String, and its escapes (`\t`, `\"`, a line-ending `\`) mean the same in either. A string holding a `\u` escape is only reported, since Groovy reads `\u0027` as a quote before it reads the string
- `GradleRemovedConfiguration`: `testCompile` becomes `testImplementation`, `runtime` becomes `runtimeOnly` and `testRuntime` becomes `testRuntimeOnly`. `compile` becomes `implementation` for most builds but `api` for a dependency a library's own API exposes, which only the build's author knows, so it is fixed only when the config's `[lint]` table names the replacement: `gradle_compile_replacement = "implementation"` (or `"api"`). Without it the report stays, and its message says so. A rename is fixed only in a file that applies a Java plugin (`java`, `java-library`, `groovy`, `scala`, `application`, `war`, `java-gradle-plugin`, Kotlin JVM, Ratpack's `io.ratpack.ratpack-java` and `-groovy`, or Android's `com.android.application`, `.library`, `.dynamic-feature` and `.test`, by `apply plugin:` or in `plugins { }`), which makes the new configurations; a file whose plugins come from a parent's `allprojects` or a convention plugin is only reported, and the message says why. `compile` becomes `api` only in a file that applies `java-library` or `com.android.library`
- `GradleJcenter`: a plain `jcenter()` line in a `repositories { }` block becomes `mavenCentral()`, or is removed when the block already has a plain `mavenCentral()`. `jcenter { ... }` is left as it is
- `GradleLeftShiftTask`: `task name << { ... }` becomes `task name { doLast { ... } }`, the action kept as written (`task 'name' << { }` is left as it is, as is one with a comment between `<<` and `{` or a line break in it)
- `GradleRemovedApi`, on a class Groovy 4 moved: `groovy.util` becomes `groovy.xml` (`groovy.ant` for `FileNameFinder`) in an import or a qualified name, and a bare `XmlSlurper`, `XmlParser` or `FileNameFinder` gets its import, `import groovy.xml.XmlSlurper`, on a line of its own after the script's last import, or before its first statement when it has none (one import for all uses of a name). Its other reports are not fixed
- `GradleSpaceAssignment`: `name value` becomes `name = value`; the check makes sure the result assigns the same text to the same property, as a statement of its own
- `GradleDependencyTuple`: the map becomes the string, `implementation 'a:b:1'`, with the values' quotes
- `GradleDependencyParentheses`: the parentheses go, `implementation 'a:b:1'`. It is reported only when each argument starts with a letter, `_` or a quote, and the fixed statement must parse as the same call with the same arguments: without parentheses, `implementation [0, 1].first()` would index a property

A Gradle fix is made only where its report fires, so the wrapper version still decides: a build pinned to a Gradle that still has `testCompile` is left alone, as is a configuration the build declares itself (`configurations { runtime }`, `create('runtime')`, `register('runtime')` and the like in that block).

A fix that fails the checks is dropped and its report stays; the file's other fixes are still made. Fixing repeats until a pass finds nothing more to fix (one pass per level of nested fixes), up to 64 passes.

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
- its tokens outside the fixed code must be the original's
- its parse tree outside the fixed code must keep the original's shape, so removing a `;` can't join two statements or turn one into another's body, and a rewritten Gradle statement can't take in its neighbor
- a removed `UnusedImport`'s name (or alias) must not be left as a name anywhere in the file outside strings, comments and other imports' names
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

## Migrating from CodeNarc

`groovyfmt --import-codenarc <FILE>...` reads a CodeNarc configuration and prints the `[lint]` table of a `.groovyfmt.toml` that carries it over, then what it couldn't carry over, as TOML comments. It writes nothing. Redirect the output to start a config:

```console
$ groovyfmt --import-codenarc config/codenarc/codenarc.xml > .groovyfmt.toml
```

It reads these formats:

- Ruleset XML: `<ruleset-ref>` with `<include>`, `<exclude>` and `<rule-config>`, and `<rule class="...">` with its properties.
- Groovy rulesets: `ruleset { }` holding `ruleset('...') { }` with `exclude` and `include`, rule names, `RuleName { property = value }` and `RuleName(property: value)`.
- `codenarc.properties`, whose `RuleName.enabled = false` lines turn rules off. Named with rulesets, it adjusts the rules they define, as in CodeNarc.
- npm-groovy-lint's `.groovylintrc.json`, with its `extends` presets and its `rules`, where `"off"`, `enabled: false`, `disabled: true` and `severity: "off"` turn a rule off. With `codenarcRulesets`, those files are read instead of `rules`, as npm-groovy-lint does.

Excludes and includes take CodeNarc's wildcards (`Empty*`) and comma-separated lists. Each groovyfmt rule named after a CodeNarc rule comes out true or false as the configuration says. A rule the configuration doesn't enable comes out false, since CodeNarc wouldn't run it. groovyfmt's Jenkins, Gradle and Spock rules, UnusedSuppression, Groovy24EmptyElement, StringLeftShiftDiscarded and NegatedInCheck keep their defaults. From a `codenarc.properties` alone, only the rules it names are set.

The report after the table lists, of the rules the configuration enables:

- the rules groovyfmt has no equivalent for yet
- the rules that need type information, which CodeNarc runs only on compiled code (its enhanced rules, the Jenkins CPS rules and a few others). They are out of scope for a tool that reads one file at a time.
- the formatting rules the formatter covers, each with what covers it. Four need no setting: Indentation (`indent`, `continuation_indent`, `use_tabs`), TrailingWhitespace, ConsecutiveBlankLines and FileEndsWithoutNewline. Fifteen spacing rules need `normalize_spacing = true`, which leaves some spacing as written ([formatting](formatting.md#left-as-written)): SpaceAfterCatch, SpaceAfterClosingBrace, SpaceAfterComma, SpaceAfterFor, SpaceAfterIf, SpaceAfterMethodCallName, SpaceAfterMethodDeclarationName, SpaceAfterOpeningBrace, SpaceAfterSwitch, SpaceAfterWhile, SpaceAroundClosureArrow, SpaceAroundOperator, SpaceBeforeClosingBrace, SpaceBeforeOpeningBrace and SpaceInsideParentheses. The settings these need (`normalize_spacing`, and `indent` from Indentation's `spacesPerIndentLevel`) are suggested as comments above `[lint]`, since they change how files are formatted. CodeNarc's other formatting rules count as having no equivalent.
- the properties set on the rules carried over (thresholds, `doNotApplyToFileNames`, `priority` and the like), which groovyfmt ignores
- whatever it couldn't interpret: custom rules, statements in a Groovy ruleset other than the forms above (a ruleset is code, and isn't run), rules CodeNarc doesn't know (renamed or removed ones), and referenced rulesets it can't or won't read

With `-v` it also prints, on stderr, each ruleset read, each rule with whether it is on and where that comes from, and every property.

### Limits

- Nothing is run and nothing is fetched.
- The rulesets CodeNarc bundles (`rulesets/basic.xml` and so on) and npm-groovy-lint's presets come from tables generated from CodeNarc 4.0.0 and npm-groovy-lint 19.0.1 by `scripts/codenarc-data.py`.
- Any other ruleset a configuration references is read only from inside the directory of the file named on the command line. It is looked up from the referencing file's directory and the directories above it, since CodeNarc reads `file:` paths from where the build runs. URLs (`http:` and the like), absolute paths and paths that lead out of that directory, symlinks included, are reported and skipped.
- The XML reader takes CodeNarc's ruleset schema only. It refuses DTDs, entities other than XML's own and character references, and nesting deeper than 8.
- Files over 1 MiB are refused, and rulesets may reference each other at most 8 deep. A directory, FIFO or device named as a ruleset is never opened.
- Each file is read once however often it is referenced. An import that would follow more than 100,000 references, or read more than 256 files or 16 MiB in all, stops with an error.
- Text from a configuration (paths, values, statements) is printed with control characters escaped and `##[` defused, so it stays inside its comment and can't pass for a CI workflow command.
- The exit status is 0 when it printed a table, and 2 when a file can't be read or isn't a CodeNarc configuration.

### On real configurations

About 120 open source projects hold 37 CodeNarc configurations in 25 of them: ruleset XML, Groovy rulesets and `.groovylintrc.json`. It reads all 37. One that includes CodeNarc's rulesets wholesale sets about 45 to 51 groovyfmt rules on and leaves 150 to 300 enabled CodeNarc rules without an equivalent, most of them naming, size, style and Javadoc rules. Eight have something it can't interpret: custom rules, rules CodeNarc has since removed (`UnnecessarySubstring`, `GrailsSessionReference`), and local variables a Groovy ruleset uses for property values.
