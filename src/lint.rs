//! Native lint rules, named after their CodeNarc equivalents so CI configs map across.
//! Files with parse errors are not linted (a broken tree would produce false positives).

use crate::text;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::{Node, Tree};

pub struct Rule {
    pub name: &'static str,
    pub default: bool,
    pub description: &'static str,
    /// Why it matters, for `--explain`.
    pub why: &'static str,
    /// Code it flags, and the same code without the problem.
    pub bad: &'static str,
    pub good: &'static str,
    /// Whether `--lint --fix` fixes it.
    pub fixable: bool,
}

/// Every rule, named after its CodeNarc equivalent so existing configs map across.
pub const RULES: &[Rule] = &[
    Rule {
        name: "UnusedImport",
        default: true,
        description: "import whose name (or alias) never appears in code or as a word in a comment",
        why: "An import nothing uses is noise: it suggests a dependency the file doesn't have, and it can hide a rename or a deleted class that would otherwise show up.",
        bad: "import java.util.concurrent.TimeUnit\nimport groovy.json.JsonSlurper\n\ndef data = new JsonSlurper().parseText(text)",
        good: "import groovy.json.JsonSlurper\n\ndef data = new JsonSlurper().parseText(text)",
        fixable: true,
    },
    Rule {
        name: "DuplicateImport",
        default: true,
        description: "the same import twice",
        why: "The second copy does nothing and usually comes from a merge; it makes the import list harder to read.",
        bad: "import groovy.json.JsonSlurper\nimport groovy.json.JsonSlurper",
        good: "import groovy.json.JsonSlurper",
        fixable: true,
    },
    Rule {
        name: "UnnecessarySemicolon",
        default: true,
        description: "`;` at the end of a line",
        why: "Groovy ends a statement at the line break, so a `;` there is Java habit and clutter.",
        bad: "def name = 'x';\nprintln name;",
        good: "def name = 'x'\nprintln name",
        fixable: true,
    },
    Rule {
        name: "EmptyCatchBlock",
        default: true,
        description: "empty catch, unless the parameter is named ignore / ignored",
        why: "Swallowing an exception hides failures. Handle or log it, or name the parameter `ignored` to say the silence is deliberate.",
        bad: "try {\n    sh 'make'\n} catch (Exception e) {\n}",
        good: "try {\n    sh 'make'\n} catch (Exception ignored) {\n    // the build step is optional\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyIfStatement",
        default: true,
        description: "if statement with an empty block",
        why: "An `if` that does nothing is dead code or an unfinished edit.",
        bad: "if (isRelease) {\n}",
        good: "if (isRelease) {\n    publish()\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyElseBlock",
        default: true,
        description: "else with an empty block",
        why: "An empty `else` adds nothing and usually marks an unfinished edit.",
        bad: "if (ok) {\n    deploy()\n} else {\n}",
        good: "if (ok) {\n    deploy()\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyTryBlock",
        default: true,
        description: "try with an empty block",
        why: "A `try` with nothing in it guards nothing; its `catch` and `finally` are dead or misplaced.",
        bad: "try {\n} finally {\n    cleanup()\n}",
        good: "cleanup()",
        fixable: false,
    },
    Rule {
        name: "EmptyFinallyBlock",
        default: true,
        description: "finally with an empty block",
        why: "An empty `finally` does nothing and usually marks forgotten cleanup.",
        bad: "try {\n    run()\n} finally {\n}",
        good: "try {\n    run()\n} finally {\n    cleanup()\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyWhileStatement",
        default: true,
        description: "while loop with an empty block",
        why: "A `while` loop with an empty block spins doing nothing, or is an unfinished edit.",
        bad: "while (queue.poll()) {\n}",
        good: "while (queue.poll()) {\n    handled++\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyForStatement",
        default: true,
        description: "for loop with an empty block",
        why: "A `for` loop with an empty block does nothing.",
        bad: "for (f in files) {\n}",
        good: "for (f in files) {\n    check(f)\n}",
        fixable: false,
    },
    Rule {
        name: "DuplicateMapKey",
        default: true,
        description: "a map literal with the same constant key twice; the later entry wins",
        why: "A map literal keeps only the last value for a key, so the earlier entry is silently lost. `a:`, `'a':` and `\"a\":` are the same key.",
        bad: "def env = [STAGE: 'dev', REGION: 'eu', STAGE: 'prod']",
        good: "def env = [STAGE: 'prod', REGION: 'eu']",
        fixable: false,
    },
    Rule {
        name: "DuplicateCaseStatement",
        default: true,
        description: "a switch with two case labels for the same constant",
        why: "The second `case` for the same value can never run.",
        bad: "switch (env) {\n    case 'prod': deploy(); break\n    case 'prod': notify(); break\n}",
        good: "switch (env) {\n    case 'prod': deploy(); notify(); break\n}",
        fixable: false,
    },
    Rule {
        name: "AssignmentInConditional",
        default: true,
        description: "`=` where `==` was probably meant, in an if condition (`while (x = next())` is a common idiom, so loops are exempt, and so is an assignment in extra parentheses)",
        why: "`if (x = 1)` assigns, and the condition is the assigned value; `==` was almost always meant. Wrap it in extra parentheses if the assignment is deliberate.",
        bad: "if (status = 'FAILED') {\n    alert()\n}",
        good: "if (status == 'FAILED') {\n    alert()\n}",
        fixable: false,
    },
    Rule {
        name: "ComparisonWithSelf",
        default: false,
        description: "a variable compared with itself (off by default: Spock equality checks do this on purpose; GORM `where { title == title }` is exempt)",
        why: "Comparing a variable with itself always gives the same answer, so the check is a typo for another variable.",
        bad: "if (expected == expected) {\n    pass()\n}",
        good: "if (actual == expected) {\n    pass()\n}",
        fixable: false,
    },
    Rule {
        name: "ConstantIfExpression",
        default: true,
        description: "an if condition that is a literal, so one branch never runs",
        why: "A literal condition means one branch never runs: a debugging leftover or dead code.",
        bad: "if (true) {\n    deploy()\n}",
        good: "deploy()",
        fixable: false,
    },
    Rule {
        name: "UnnecessaryGString",
        default: false,
        description: "double-quoted string with nothing to interpolate (off by default: noisy where double quotes are the house style)",
        why: "A double-quoted string without `$` reads as if something were interpolated; a single-quoted String says it isn't.",
        bad: "def stage = \"build\"",
        good: "def stage = 'build'",
        fixable: false,
    },
    // Suppressions (src/lint/suppress.rs)
    Rule {
        name: "UnusedSuppression",
        default: false,
        description: "a groovyfmt-ignore comment that suppresses nothing, or a codenarc-disable comment or @SuppressWarnings naming a groovyfmt rule it doesn't suppress (off by default: a suppression can be kept for code that comes and goes)",
        why: "A suppression that matches nothing hides the next real report on that line, and a misspelled rule name in it suppresses nothing at all.",
        bad: "// groovyfmt-ignore UnusedImport\nprintln 'hello'",
        good: "import a.Unused // groovyfmt-ignore UnusedImport -- loaded for its side effect\nprintln 'hello'",
        fixable: false,
    },
    // Imports (src/lint/imports.rs)
    Rule {
        name: "UnnecessaryGroovyImport",
        default: true,
        description: "an import from java.lang, java.util, java.io, java.net, groovy.lang or groovy.util, or of BigDecimal or BigInteger, which Groovy imports by default (not in *.gradle files)",
        why: "Groovy imports these packages into every file, so the import is noise. It can be needed only to beat a class of the same name in the file's own package, which is why --fix leaves it to you.",
        bad: "import java.util.concurrent.TimeUnit\nimport java.util.List\n\nList<TimeUnit> units = []",
        good: "import java.util.concurrent.TimeUnit\n\nList<TimeUnit> units = []",
        fixable: false,
    },
    Rule {
        name: "ImportFromSamePackage",
        default: true,
        description: "an import of a class from the file's own package",
        why: "Classes of the file's own package need no import: Groovy looks there right after the imports, so the line does nothing.",
        bad: "package com.example.build\n\nimport com.example.build.Versions\n\ndef v = Versions.KOTLIN",
        good: "package com.example.build\n\ndef v = Versions.KOTLIN",
        fixable: true,
    },
    // Bugs the syntax shows (src/lint/basic.rs)
    Rule {
        name: "DeadCode",
        default: true,
        description: "a statement after return, throw, break or continue in the same block, on a later line (not a declaration, a labeled statement, or a break right after a return or throw)",
        why: "Code after a `return`, `throw`, `break` or `continue` never runs: it is left over from an edit, or the jump is in the wrong place.",
        bad: "def check(x) {\n    return x > 0\n    log.info 'checked'\n}",
        good: "def check(x) {\n    log.info 'checked'\n    return x > 0\n}",
        fixable: false,
    },
    Rule {
        name: "DuplicateSetValue",
        default: false,
        description: "a list literal made a Set (`[...] as Set`) with the same constant twice (off by default: tests repeat values on purpose to check that a set drops them)",
        why: "A set keeps one copy of each value, so the second copy is dropped: usually a typo for another value.",
        bad: "def stages = ['build', 'test', 'build'] as Set",
        good: "def stages = ['build', 'test', 'deploy'] as Set",
        fixable: false,
    },
    Rule {
        name: "EqualsAndHashCode",
        default: true,
        description: "a class defining equals(Object) without hashCode()",
        why: "Equal objects must have equal hash codes; with a custom equals and the inherited identity hashCode, equal objects land in different buckets, so maps and sets misbehave.",
        bad: "class Point {\n    int x\n    boolean equals(Object o) { o instanceof Point && o.x == x }\n}",
        good: "class Point {\n    int x\n    boolean equals(Object o) { o instanceof Point && o.x == x }\n    int hashCode() { x }\n}",
        fixable: false,
    },
    Rule {
        name: "EqualsOverloaded",
        default: true,
        description: "an equals method whose one parameter is typed other than Object, so it overloads equals instead of overriding it",
        why: "`equals(Point p)` is a new method, not an override: `==`, collections and maps still call `equals(Object)` and never see it.",
        bad: "class Point {\n    int x\n    boolean equals(Point p) { p.x == x }\n    int hashCode() { x }\n}",
        good: "class Point {\n    int x\n    boolean equals(Object o) { o instanceof Point && o.x == x }\n    int hashCode() { x }\n}",
        fixable: false,
    },
    Rule {
        name: "ReturnFromFinallyBlock",
        default: true,
        description: "a return in a finally block",
        why: "A `return` in `finally` replaces whatever the `try` returned and silently discards any exception it threw.",
        bad: "def read() {\n    try {\n        return load()\n    } finally {\n        return null\n    }\n}",
        good: "def read() {\n    try {\n        return load()\n    } finally {\n        close()\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "ThrowExceptionFromFinallyBlock",
        default: false,
        description: "a throw in a finally block (not one a catch inside the finally handles) (off by default: rethrowing a saved exception after cleanup is a common idiom)",
        why: "An exception thrown from `finally` replaces the one the `try` threw, which is the one that explains the failure.",
        bad: "try {\n    run()\n} finally {\n    throw new IllegalStateException('cleanup failed')\n}",
        good: "try {\n    run()\n} finally {\n    cleanup()\n}",
        fixable: false,
    },
    Rule {
        name: "AssertWithinFinallyBlock",
        default: false,
        description: "an assert in a finally block (off by default: tests check state after cleanup this way on purpose)",
        why: "A failing `assert` in `finally` throws and replaces the exception the `try` may have thrown.",
        bad: "try {\n    run()\n} finally {\n    assert done\n}",
        good: "try {\n    run()\n} finally {\n    cleanup()\n}\nassert done",
        fixable: false,
    },
    Rule {
        name: "EmptySwitchStatement",
        default: true,
        description: "a switch with no cases",
        why: "A `switch` with no cases does nothing: an unfinished edit.",
        bad: "switch (env) {\n}",
        good: "switch (env) {\n    case 'prod': deploy(); break\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyStaticInitializer",
        default: true,
        description: "an empty static { } block in a class",
        why: "An empty static initializer does nothing and can be removed.",
        bad: "class Config {\n    static {\n    }\n}",
        good: "class Config {\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyInstanceInitializer",
        default: true,
        description: "an empty { } instance initializer in a class",
        why: "An empty instance initializer does nothing and can be removed.",
        bad: "class Config {\n    {\n    }\n}",
        good: "class Config {\n}",
        fixable: false,
    },
    Rule {
        name: "ConstantTernaryExpression",
        default: false,
        description: "a ternary or elvis whose condition is a constant, so it always picks the same branch (off by default: on real code it found only tests of the ternary itself)",
        why: "With a constant condition, `true ? a : b` is always `a`: a debugging leftover or a typo for a variable.",
        bad: "def level = true ? 'debug' : 'info'",
        good: "def level = verbose ? 'debug' : 'info'",
        fixable: false,
    },
    Rule {
        name: "ConstantAssertExpression",
        default: false,
        description: "an assert whose condition is a constant (off by default: tests use `assert false` and `assert true` on purpose)",
        why: "`assert true` always passes and checks nothing; `assert null` always fails. Assert the condition that should hold.",
        bad: "assert true",
        good: "assert result.ok",
        fixable: false,
    },
    Rule {
        name: "ComparisonOfTwoConstants",
        default: false,
        description: "a comparison of two constants or literals made of constants (off by default: tests and placeholders (`expect: 1 == 1`) do it on purpose)",
        why: "Comparing two constants always gives the same answer, so one side was meant to be a variable.",
        bad: "if ('prod' == 'prod') {\n    deploy()\n}",
        good: "if (env == 'prod') {\n    deploy()\n}",
        fixable: false,
    },
    Rule {
        name: "BrokenNullCheck",
        default: true,
        description: "`x != null || x.a` or `x == null && x.a`, which dereference x exactly when it is null",
        why: "The right side runs only when `x` is null, so it throws a NullPointerException: `&&` and `||` are swapped.",
        bad: "if (name != null || name.length() > 0) {\n    greet(name)\n}",
        good: "if (name != null && name.length() > 0) {\n    greet(name)\n}",
        fixable: false,
    },
    Rule {
        name: "BrokenOddnessCheck",
        default: false,
        description: "`x % 2 == 1`, which is false for negative odd numbers (off by default: almost every real case is a count or size that is never negative)",
        why: "For a negative odd `x`, `x % 2` is -1, so the check says it is even. `x % 2 != 0` works for both signs.",
        bad: "if (n % 2 == 1) {\n    odd++\n}",
        good: "if (n % 2 != 0) {\n    odd++\n}",
        fixable: false,
    },
    Rule {
        name: "RemoveAllOnSelf",
        default: true,
        description: "`c.removeAll(c)`",
        why: "`c.removeAll(c)` empties the collection less clearly than `c.clear()`, slower, and some collections throw ConcurrentModificationException.",
        bad: "pending.removeAll(pending)",
        good: "pending.clear()",
        fixable: false,
    },
    Rule {
        name: "ObjectOverrideMisspelledMethodName",
        default: true,
        description: "equal(Object), hashcode(), toSTring() and other misspellings of equals, hashCode and toString",
        why: "A misspelled `hashcode()` or `equal(Object)` doesn't override Object's method, so nothing calls it.",
        bad: "class Point {\n    int x\n    int hashcode() { x }\n}",
        good: "class Point {\n    int x\n    int hashCode() { x }\n}",
        fixable: false,
    },
    Rule {
        name: "ToStringReturnsNull",
        default: false,
        description: "a toString() that returns null, ends in null, or is empty (off by default: on real code it found only test fixtures that return null on purpose)",
        why: "Callers expect `toString()` to return text; null breaks string building and logging with a NullPointerException.",
        bad: "class Job {\n    String name\n    String toString() {\n        return null\n    }\n}",
        good: "class Job {\n    String name\n    String toString() {\n        return \"Job ${name}\"\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "MissingNewInThrowStatement",
        default: true,
        description: "`throw SomeException(...)` or `throw SomeException` without new (a capitalized name ending in Exception, Failure or Fault)",
        why: "Without `new`, Groovy calls a method named `RuntimeException`, which doesn't exist, so the error path throws MissingMethodException and hides the real cause.",
        bad: "throw IllegalStateException('no target')",
        good: "throw new IllegalStateException('no target')",
        fixable: false,
    },
    Rule {
        name: "ExceptionNotThrown",
        default: true,
        description: "a catch block ending in `new SomeException(...)` that isn't thrown (or returned as the method's value)",
        why: "Creating an exception does nothing until it is thrown; without `throw` the error is swallowed.",
        bad: "void run() {\n    try {\n        work()\n    } catch (IOException e) {\n        new IllegalStateException(e)\n    }\n}",
        good: "void run() {\n    try {\n        work()\n    } catch (IOException e) {\n        throw new IllegalStateException(e)\n    }\n}",
        fixable: false,
    },
    // Unused code (src/lint/unused.rs)
    Rule {
        name: "UnusedPrivateField",
        default: true,
        description: "a private field whose name appears nowhere else in the file (not one with an annotation, serialVersionUID, or in a class whose annotation reads fields)",
        why: "A private field nothing reads is dead state: left over from a refactoring, or a sign that the code meant to use it doesn't.",
        bad: "class Client {\n    private int retries = 3\n    def get(url) { url.toURL().text }\n}",
        good: "class Client {\n    def get(url) { url.toURL().text }\n}",
        fixable: false,
    },
    Rule {
        name: "UnusedPrivateMethod",
        default: true,
        description: "a private method whose name appears nowhere else in the file (for getX/isX/setX, nor the property x)",
        why: "A private method nothing calls is dead code that still has to be read and maintained.",
        bad: "class Client {\n    def get(url) { url.toURL().text }\n    private String trimmed(s) { s.trim() }\n}",
        good: "class Client {\n    def get(url) { url.toURL().text }\n}",
        fixable: false,
    },
    Rule {
        name: "UnusedVariable",
        default: true,
        description: "a local variable whose name never appears after its declaration in its block (not a script's top-level variable, `_`, an annotated one, or a `def (a, b)` tuple)",
        why: "A variable nothing reads is dead code, or the sign of a typo: the code meant to use it and uses something else.",
        bad: "def total(items) {\n    def count = items.size()\n    items.sum()\n}",
        good: "def total(items) {\n    items.sum()\n}",
        fixable: false,
    },
    // Declarative Jenkins pipelines (src/lint/jenkins.rs)
    Rule {
        name: "JenkinsMissingSection",
        default: true,
        description: "pipeline without agent or stages, stages without a stage, a stage with none of steps, stages, parallel, matrix",
        why: "A declarative pipeline needs `agent` and `stages`, each `stages` needs a stage, and each stage needs `steps`, `stages`, `parallel` or `matrix`; Jenkins rejects the file when the build starts, after the push.",
        bad: "pipeline {\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsConflictingSections",
        default: true,
        description: "a section twice in one block, or a stage with more than one of steps, stages, parallel, matrix",
        why: "Jenkins rejects a section given twice in one block, and a stage with more than one of `steps`, `stages`, `parallel` and `matrix`, when the build starts.",
        bad: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n            steps {\n                sh 'make test'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n                sh 'make test'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsUnknownSection",
        default: true,
        description: "a section that doesn't belong where it is in pipeline, stage, matrix, post or stages",
        why: "A section in the wrong place (`steps` directly in `pipeline`, `parameters` in a stage) or a misspelled one fails validation when the build starts.",
        bad: "pipeline {\n    agent any\n    steps {\n        sh 'make'\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsStatementOutsideSection",
        default: true,
        description: "Groovy code directly in pipeline, stages, stage, matrix, post or when instead of in steps",
        why: "Declarative pipelines only allow sections where Groovy code would otherwise go; code belongs in `steps` (inside `script { }` when it isn't a step) or in `environment`.",
        bad: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            def target = 'all'\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsScriptRequired",
        default: true,
        description: "if, for, while, try, switch, def or an assignment directly in steps without script { }",
        why: "`steps` takes steps only: `if`, loops, `try`, `def` and assignments must be wrapped in `script { }`, or the build fails to start.",
        bad: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            steps {\n                if (env.BRANCH_NAME == 'main') {\n                    sh 'make deploy'\n                }\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            steps {\n                script {\n                    if (env.BRANCH_NAME == 'main') {\n                        sh 'make deploy'\n                    }\n                }\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsUnknownWhenCondition",
        default: true,
        description: "a when condition that isn't built in",
        why: "A `when` condition that isn't built in (a typo such as `branchName`) fails validation when the build starts.",
        bad: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            when {\n                branchName 'main'\n            }\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            when {\n                branch 'main'\n            }\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsMisplacedOption",
        default: true,
        description: "parameters or triggers in options, or a pipeline-wide option in a stage's options",
        why: "`parameters` and `triggers` are sections of their own, not options, and some options apply only to the whole pipeline; Jenkins rejects them in the wrong place.",
        bad: "pipeline {\n    agent any\n    options {\n        triggers {\n            cron('H 2 * * *')\n        }\n    }\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    triggers {\n        cron('H 2 * * *')\n    }\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsDuplicateStageName",
        default: true,
        description: "two stages with the same name in one stages or parallel block",
        why: "Stage names must be unique within their block; Jenkins rejects duplicates, and the stage view can't tell them apart.",
        bad: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n        stage('Build') {\n            steps {\n                sh 'make test'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n        stage('Test') {\n            steps {\n                sh 'make test'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsStageName",
        default: true,
        description: "a stage with no name, or a name that isn't a plain string",
        why: "A declarative stage needs a name that is a plain string; an interpolated name (`\"Deploy ${env}\"`) is rejected.",
        bad: "pipeline {\n    agent any\n    stages {\n        stage(\"Deploy ${params.TARGET}\") {\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            steps {\n                sh \"make deploy TARGET=${params.TARGET}\"\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsAgentForm",
        default: true,
        description: "agent with no value, an empty or multi-type block, or a bare type other than any and none",
        why: "`agent` takes `any`, `none` or a block naming one agent type; anything else fails validation when the build starts.",
        bad: "pipeline {\n    agent docker\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent {\n        docker {\n            image 'gcc:13'\n        }\n    }\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    // Gradle build scripts (src/lint/gradle.rs)
    Rule {
        name: "GradleRemovedConfiguration",
        default: true,
        description: "compile, runtime, testCompile or testRuntime dependencies (removed in Gradle 7.0)",
        why: "`compile`, `runtime`, `testCompile` and `testRuntime` were removed in Gradle 7.0; the build fails on a current Gradle.",
        bad: "dependencies {\n    compile 'org.slf4j:slf4j-api:2.0.9'\n}",
        good: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.0.9'\n}",
        fixable: false,
    },
    Rule {
        name: "GradleJcenter",
        default: true,
        description: "the jcenter() repository (removed in Gradle 9.0)",
        why: "JCenter is read-only and `jcenter()` was removed in Gradle 9.0.",
        bad: "repositories {\n    jcenter()\n}",
        good: "repositories {\n    mavenCentral()\n}",
        fixable: false,
    },
    Rule {
        name: "GradleLeftShiftTask",
        default: true,
        description: "task name << { } (removed in Gradle 5.0)",
        why: "`task name << { }` was removed in Gradle 5.0; the build fails on a current Gradle.",
        bad: "task hello << {\n    println 'hello'\n}",
        good: "tasks.register('hello') {\n    doLast {\n        println 'hello'\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "GradleDynamicVersion",
        default: false,
        description: "a + or latest.* dependency version (off by default: a policy choice)",
        why: "A `+` or `latest.*` version resolves to whatever is newest at build time, so the same commit can build differently tomorrow.",
        bad: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.+'\n}",
        good: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.0.9'\n}",
        fixable: false,
    },
    // Spock specifications (src/lint/spock.rs)
    Rule {
        name: "SpockBlockOrder",
        default: true,
        description: "feature method blocks in an order Spock rejects, or a label Spock doesn't know",
        why: "Spock refuses to compile feature blocks out of order (`then:` without a `when:` before it), so the spec fails to build.",
        bad: "class MathSpec extends Specification {\n    def 'adds'() {\n        then:\n        1 + 1 == 2\n    }\n}",
        good: "class MathSpec extends Specification {\n    def 'adds'() {\n        expect:\n        1 + 1 == 2\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "SpockEmptyBlock",
        default: false,
        description: "an empty then: or expect: block (off by default: Spock allows it, and its own tests use empty blocks on purpose)",
        why: "A `then:` or `expect:` block with only a description checks nothing, so the feature passes whatever happens.",
        bad: "class MathSpec extends Specification {\n    def 'runs'() {\n        when:\n        run()\n        then: 'it does not throw'\n    }\n}",
        good: "class MathSpec extends Specification {\n    def 'runs'() {\n        when:\n        run()\n        then: 'it does not throw'\n        noExceptionThrown()\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "SpockDataTableColumns",
        default: true,
        description: "a data table row with a different number of columns than the row above",
        why: "A data table row with a missing or extra cell is a compile error, or shifts the values into the wrong variables.",
        bad: "class MathSpec extends Specification {\n    def 'max'() {\n        expect:\n        Math.max(a, b) == c\n        where:\n        a | b | c\n        1 | 2\n    }\n}",
        good: "class MathSpec extends Specification {\n    def 'max'() {\n        expect:\n        Math.max(a, b) == c\n        where:\n        a | b | c\n        1 | 2 | 2\n    }\n}",
        fixable: false,
    },
];

mod basic;
mod gradle;
mod imports;
mod jenkins;
mod spock;
mod suppress;
mod unused;

#[derive(Debug, PartialEq)]
pub struct Violation {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based, in characters
    /// Where the flagged node ends: the line, and the column just after its last character.
    pub end_line: usize,
    pub end_col: usize,
    pub rule: &'static str,
    pub message: String,
    /// The bytes `--fix` deletes to fix it, for the rules it fixes: the `;`, or the import's
    /// whole lines when nothing else is on them.
    pub fix: Option<std::ops::Range<usize>>,
    /// The bytes of the flagged code, so `--fix` can tell a report that goes with a deleted
    /// line (an unused import that is also unnecessary) from one that changed.
    pub span: std::ops::Range<usize>,
}

pub fn lint(src: &str) -> Result<Vec<Violation>, &'static str> {
    lint_file(Path::new(""), src)
}

/// Lint `src` as the file at `path`, whose name decides the Jenkinsfile and Gradle rules.
pub fn lint_file(path: &Path, src: &str) -> Result<Vec<Violation>, &'static str> {
    let tree = crate::parse(src);
    if tree.root_node().has_error() {
        return Err("parse error");
    }
    let mut v = Vec::new();
    imports(src, &tree, &mut v);
    jenkins::check(path, src, tree.root_node(), &mut v);
    gradle::check(path, src, tree.root_node(), &mut v);
    let mut nodes = Vec::new();
    crate::preorder(tree.root_node(), |n| {
        nodes.push(n);
        true
    });
    // the nodes each of basic, unused and imports looks at, in preorder
    let table = kinds(&tree.language());
    let mut by_module: [Vec<Node>; 6] = Default::default();
    for (i, &n) in nodes.iter().enumerate() {
        let (kind, m) = table
            .get(usize::from(n.kind_id()))
            .copied()
            .unwrap_or(("", 0));
        if m != 0 {
            for (j, list) in by_module.iter_mut().enumerate() {
                if m & (1 << j) != 0 {
                    list.push(n);
                }
            }
        }
        match kind {
            "string_literal" => unnecessary_gstring(src, n, &mut v),
            ";" => unnecessary_semicolon(n, nodes.get(i + 1..).unwrap_or(&[]), &mut v),
            "catch_clause" => empty_catch(src, n, &mut v),
            "if_statement" => if_statement(n, &mut v),
            "while_statement" => {
                empty_body(
                    n,
                    "body",
                    "EmptyWhileStatement",
                    "The while statement is empty",
                    &mut v,
                );
            }
            "for_statement" | "for_in_statement" => empty_body(
                n,
                "body",
                "EmptyForStatement",
                "The for statement is empty",
                &mut v,
            ),
            "try_statement" => {
                empty_body(n, "body", "EmptyTryBlock", "The try block is empty", &mut v)
            }
            "finally_clause" => empty_body(
                n,
                "body",
                "EmptyFinallyBlock",
                "The finally block is empty",
                &mut v,
            ),
            "map_literal" => duplicate_map_key(src, n, &mut v),
            "switch_block" => duplicate_case(src, n, &mut v),
            "binary_expression" => comparison_with_self(src, n, &mut v),
            "class_declaration" => spock::class(src, n, &mut v),
            _ => {}
        }
    }
    let [
        basic_nodes,
        unused_nodes,
        import_nodes,
        name_leaves,
        dynamic,
        directives,
    ] = by_module;
    basic::check(src, &basic_nodes, &mut v);
    unused::check(src, &unused_nodes, &name_leaves, &dynamic, &mut v);
    imports::check(path, src, &import_nodes, &mut v);
    suppress::apply(src, &directives, &mut v);
    // tree-sitter columns count bytes; report characters, as editors and GitHub do
    let lines: Vec<&str> = src.split('\n').collect();
    let chars = |line: usize, col: usize| {
        lines
            .get(line - 1)
            .and_then(|l| l.get(..col - 1))
            .map_or(col, |prefix| prefix.chars().count() + 1)
    };
    for x in &mut v {
        x.col = chars(x.line, x.col);
        x.end_col = chars(x.end_line, x.end_col);
    }
    v.sort_by_key(|x| (x.line, x.col));
    Ok(v)
}

/// By node kind id: the kind's name, and a bit for each list in [basic, unused, imports,
/// unused's name leaves, unused's dynamic member access, suppressions] whose kinds include it. Looking the
/// name up here costs one call into tree-sitter per node instead of two, and each module
/// sees only its own nodes: a pass over every node per module cost 25% of `--lint`.
fn kinds(lang: &tree_sitter::Language) -> &'static [(&'static str, u8)] {
    static TABLE: std::sync::OnceLock<Vec<(&'static str, u8)>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let modules = [
            basic::KINDS,
            unused::KINDS,
            imports::KINDS,
            unused::LEAVES,
            unused::DYNAMIC,
            suppress::KINDS,
        ];
        (0..lang.node_kind_count())
            .map(|id| {
                let id = u16::try_from(id).unwrap_or(u16::MAX);
                let kind = lang.node_kind_for_id(id).unwrap_or("");
                let named = lang.node_kind_is_named(id);
                let bits = modules
                    .iter()
                    .enumerate()
                    .filter(|(_, ks)| named && ks.contains(&kind))
                    .fold(0, |m, (i, _)| m | 1 << i);
                (kind, bits)
            })
            .collect()
    })
}

fn at(n: Node, rule: &'static str, message: String) -> Violation {
    let (p, e) = (n.start_position(), n.end_position());
    Violation {
        line: p.row + 1,
        col: p.column + 1,
        end_line: e.row + 1,
        end_col: e.column + 1,
        rule,
        message,
        fix: None,
        span: n.byte_range(),
    }
}

/// The lines holding `n`, with the line break after them, when nothing but whitespace and
/// a `;` shares them.
fn own_lines(src: &str, n: Node) -> Option<std::ops::Range<usize>> {
    let start = src[..n.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let end = src[n.end_byte()..]
        .find('\n')
        .map_or(src.len(), |i| n.end_byte() + i + 1);
    let after = src[n.end_byte()..end].trim();
    (src[start..n.start_byte()].trim().is_empty() && (after.is_empty() || after == ";"))
        .then_some(start..end)
}

/// UnusedImport + DuplicateImport.
fn imports(src: &str, tree: &Tree, v: &mut Vec<Violation>) {
    let (mut decls, mut used) = (Vec::new(), HashSet::new());
    crate::preorder(tree.root_node(), |n| match n.kind() {
        // don't descend: names inside imports aren't uses
        "import_declaration" => {
            decls.push(n);
            false
        }
        // a whole word in a comment ({@link Foo}) counts as a use, as in CodeNarc
        k if k.contains("comment") => {
            used.extend(words(text(n, src)));
            false
        }
        // `"$Name.CONST"` is one leaf whose text keeps the `$` and the dots
        "gstring_dollar_interpolation" => {
            used.extend(text(n, src).trim_start_matches('$').split('.'));
            false
        }
        _ => {
            if n.child_count() == 0 {
                let t = text(n, src);
                used.insert(t);
                // The grammar sometimes leaves a GString's interpolations as plain text, as in
                // `"""` holding `"${Name.x}"`: count the names after each unescaped `$`.
                if n.kind() == "string_fragment"
                    && t.contains('$')
                    && n.parent().is_some_and(|p| !text(p, src).starts_with('\''))
                {
                    dollar_names(t, &mut used);
                }
            }
            true
        }
    });
    let mut seen = HashSet::new();
    for d in decls {
        let full = text(d, src)
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // `@Library('lib') import x` and `@Grab(...) import y` are there for the annotation:
        // never unused, and never removed
        let annotated = text(d, src).starts_with('@');
        let fix = own_lines(src, d).filter(|_| !annotated);
        if !seen.insert(full.clone()) {
            v.push(Violation {
                fix,
                ..at(d, "DuplicateImport", format!("Duplicate import: {full}"))
            });
            continue;
        }
        let name = d.child_by_field_name("name").map_or("", |n| text(n, src));
        if name.ends_with('*') || full.ends_with(".*") || annotated {
            continue;
        }
        let symbol = d
            .child_by_field_name("alias")
            .map_or_else(|| name.rsplit('.').next().unwrap_or(name), |a| text(a, src));
        if !symbol.is_empty() && !used.contains(symbol) {
            v.push(Violation {
                fix,
                ..at(
                    d,
                    "UnusedImport",
                    format!("The [{name}] import is never referenced"),
                )
            });
        }
    }
}

/// The identifier-like words in `s`.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

/// The names after each unescaped `$` in GString text: `$a.b`, and every word in `${...}`.
fn dollar_names<'a>(t: &'a str, used: &mut HashSet<&'a str>) {
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'$' => {
                let start = i + 1;
                let end = if b.get(start) == Some(&b'{') {
                    // to the matching `}`, or the end of the text
                    let mut depth = 0;
                    (start..b.len())
                        .find(|&j| {
                            depth += i32::from(b[j] == b'{') - i32::from(b[j] == b'}');
                            depth == 0
                        })
                        .unwrap_or(b.len())
                } else {
                    t[start..]
                        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                        .map_or(b.len(), |k| start + k)
                };
                used.extend(words(&t[start..end]));
                i = end;
            }
            _ => i += 1,
        }
    }
}

/// UnnecessaryGString: double-quoted string with nothing to interpolate.
fn unnecessary_gstring(src: &str, n: Node, v: &mut Vec<Violation>) {
    let t = text(n, src);
    if t.starts_with('"') && !has_interpolation(n) && !t.contains('\'') && !t.contains('$') {
        v.push(at(
            n,
            "UnnecessaryGString",
            format!("The String {t} can be wrapped in single quotes instead of double quotes"),
        ));
    }
}

/// UnnecessarySemicolon: a `;` that ends a line (or the file). Not one that is a loop's
/// empty body (`while (it.next());`), or one before a line starting with `{`
/// (`foo();` then `{ -> x }.call()`) or `++` / `--` (`a[0];` then `++n`): either way
/// removing it would change the program, here by making the closure an argument of the
/// statement before, or the `++` a postfix increment of it.
fn unnecessary_semicolon(n: Node, rest: &[Node], v: &mut Vec<Violation>) {
    if n.parent().is_some_and(|p| {
        p.kind().contains("enum")
            || p.kind() == "for_statement"
            || (p.kind() == "while_statement" && p.child_by_field_name("body") == Some(n))
    }) {
        return;
    }
    let row = n.start_position().row;
    // next leaf in document order, ignoring comments
    let next = rest
        .iter()
        .find(|x| x.child_count() == 0 && !x.kind().contains("comment"));
    if next.is_none_or(|x| x.start_position().row > row && !matches!(x.kind(), "{" | "++" | "--")) {
        v.push(Violation {
            fix: Some(n.byte_range()),
            ..at(
                n,
                "UnnecessarySemicolon",
                "Semicolons as line endings can be removed safely".into(),
            )
        });
    }
}

/// EmptyCatchBlock, with CodeNarc's default exemption for `ignore` / `ignored` parameter names.
fn empty_catch(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else {
        return;
    };
    let param = n
        .child_by_field_name("parameter")
        .map_or("", |p| text(p, src));
    let ignored = param
        .split_whitespace()
        .last()
        .is_some_and(|w| w == "ignore" || w == "ignored");
    if body.named_child_count() == 0 && !ignored {
        v.push(at(n, "EmptyCatchBlock", "The catch block is empty".into()));
    }
}

fn is_empty_block(n: Option<Node>) -> bool {
    // a comment inside counts as content, as an explanation of the emptiness
    n.is_some_and(|b| b.kind() == "block" && b.named_child_count() == 0)
}

fn empty_body(n: Node, field: &str, rule: &'static str, msg: &str, v: &mut Vec<Violation>) {
    if is_empty_block(n.child_by_field_name(field)) {
        v.push(at(n, rule, msg.into()));
    }
}

/// Condition with any wrapping parentheses removed.
fn condition(n: Node) -> Option<Node> {
    let mut c = n.child_by_field_name("condition")?;
    while c.kind() == "parenthesized_expression" {
        c = c.named_child(0)?;
    }
    Some(c)
}

fn is_literal(n: Node) -> bool {
    matches!(
        n.kind(),
        "boolean_literal" | "null_literal" | "number_literal" | "string_literal"
    ) && !has_interpolation(n)
}

fn has_interpolation(n: Node) -> bool {
    let mut c = n.walk();
    n.named_children(&mut c)
        .any(|k| k.kind().starts_with("gstring_"))
}

/// AssignmentInConditional. Extra parentheses, `if ((m = re.matcher(s)))`, mark the
/// assignment as meant, as with GCC's -Wparentheses and ESLint's no-cond-assign.
fn assignment_in_condition(n: Node, v: &mut Vec<Violation>) {
    if let Some(c) = n.child_by_field_name("condition")
        && c.kind() == "assignment_expression"
        && c.child_by_field_name("operator")
            .is_some_and(|o| o.kind() == "=")
    {
        v.push(at(
            c,
            "AssignmentInConditional",
            "Assignment used as the if condition; did you mean ==?".into(),
        ));
    }
}

/// EmptyIfStatement, EmptyElseBlock, ConstantIfExpression, AssignmentInConditional.
fn if_statement(n: Node, v: &mut Vec<Violation>) {
    empty_body(
        n,
        "consequence",
        "EmptyIfStatement",
        "The if statement is empty",
        v,
    );
    if let Some(alt) = n.child_by_field_name("alternative")
        && is_empty_block(Some(alt))
    {
        v.push(at(alt, "EmptyElseBlock", "The else block is empty".into()));
    }
    if let Some(c) = condition(n)
        && is_literal(c)
    {
        v.push(at(
            c,
            "ConstantIfExpression",
            "The if condition is a constant, so one branch never runs".into(),
        ));
    }
    assignment_in_condition(n, v);
}

/// The value a constant map key or case label stands for: `a`, `'a'` and `"a"` are the same
/// map key in Groovy. `None` for anything that isn't a constant.
fn constant_value(n: Node, src: &str) -> Option<String> {
    match n.kind() {
        "identifier" | "number_literal" | "boolean_literal" | "null_literal" => {
            Some(text(n, src).to_string())
        }
        "string_literal" if !has_interpolation(n) => {
            let t = text(n, src);
            let q = ["'''", "\"\"\"", "'", "\""]
                .iter()
                .find(|q| t.starts_with(**q) && t.ends_with(**q) && t.len() >= 2 * q.len())?;
            Some(t[q.len()..t.len() - q.len()].to_string())
        }
        _ => None,
    }
}

fn duplicate_map_key(src: &str, n: Node, v: &mut Vec<Violation>) {
    let mut seen = HashSet::new();
    let mut c = n.walk();
    for e in n.named_children(&mut c).filter(|e| e.kind() == "map_entry") {
        let Some(key) = e.child_by_field_name("key") else {
            continue;
        };
        // only identifier and string keys share a meaning; numbers stay distinct from strings
        let k = constant_value(key, src).map(|val| (key.kind() == "number_literal", val));
        if let Some(k) = k
            && !seen.insert(k.clone())
        {
            v.push(at(
                key,
                "DuplicateMapKey",
                format!("Key `{}` appears more than once in this map", k.1),
            ));
        }
    }
}

fn duplicate_case(src: &str, n: Node, v: &mut Vec<Violation>) {
    let mut seen = HashSet::new();
    let mut c = n.walk();
    for case in n
        .named_children(&mut c)
        .filter(|k| k.kind() == "switch_case")
    {
        let Some(val) = case.child_by_field_name("value") else {
            continue;
        };
        if !is_literal(val) {
            continue;
        }
        // `'a'` and `"a"` are the same label; `1` and `'1'` are not
        let k = constant_value(val, src).map(|s| (val.kind() == "string_literal", s));
        if let Some(k) = k
            && !seen.insert(k)
        {
            v.push(at(
                val,
                "DuplicateCaseStatement",
                format!("Duplicate case label {}", text(val, src)),
            ));
        }
    }
}

fn comparison_with_self(src: &str, n: Node, v: &mut Vec<Violation>) {
    let op = n.child_by_field_name("operator").map_or("", |o| o.kind());
    if !matches!(
        op,
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "<=>" | "===" | "!=="
    ) {
        return;
    }
    let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) else {
        return;
    };
    // plain variables only: `f() == f()` can legitimately differ
    if l.kind() == "identifier"
        && r.kind() == "identifier"
        && text(l, src) == text(r, src)
        && !in_where_query(n, src)
    {
        v.push(at(
            n,
            "ComparisonWithSelf",
            format!("Comparing `{}` with itself", text(l, src)),
        ));
    }
}

/// Inside a closure given to a GORM where query (`Book.where { title == title }`,
/// `@Where({ ... })`), where the left side names the domain property and the right a
/// variable of the same name.
fn in_where_query(n: Node, src: &str) -> bool {
    let mut in_closure = false;
    let mut p = n.parent();
    while let Some(a) = p {
        match a.kind() {
            "closure" => in_closure = true,
            "method_invocation" | "command_chain" | "annotation" if in_closure => {
                let head = text(a, src).split(['(', '{']).next().unwrap_or("");
                let name = head.trim().rsplit(['.', '@']).next().unwrap_or("");
                if matches!(name, "where" | "whereAny" | "whereLazy" | "Where") {
                    return true;
                }
            }
            _ => {}
        }
        p = a.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(src: &str) -> Vec<(&'static str, usize)> {
        lint(src)
            .expect("parses")
            .into_iter()
            .map(|v| (v.rule, v.line))
            .collect()
    }

    #[test]
    fn imports_unused_duplicate_and_used() {
        let src = "import a.b.Used\nimport a.b.Unused\nimport a.b.Used\nimport a.c.*\nimport a.b.Linked\n\n/** see {@link Linked} */\nclass X { Used u }\n";
        assert_eq!(
            rules(src),
            vec![("UnusedImport", 2), ("DuplicateImport", 3)]
        );
    }

    #[test]
    fn dollar_interpolation_is_a_use() {
        for s in [
            "\"$N\"",
            "\"${M.X} $N\"",
            "\"$N.CONST\"",
            "\"$N.a.b\"",
            "\"\"\"x\n$N y\"\"\"",
            "/x $N y/",
            "$/x $N y/$",
        ] {
            let src = format!("import a.N\nimport a.M\nprintln {s}\nM.go()\n");
            assert_eq!(rules(&src), vec![], "{s}");
        }
        // escaped, or in a string that doesn't interpolate: not a use
        for s in ["\"\\$N\"", "'$N'", "'''$N'''", "\"$Nx\""] {
            let src = format!("import a.N\nprintln {s}\n");
            assert_eq!(rules(&src), vec![("UnusedImport", 1)], "{s}");
        }
    }

    #[test]
    fn interpolation_the_grammar_left_as_text_is_a_use() {
        // gradle ExternalProcessFixture: in this `"""` the `${...}` parses as plain text
        let src = "import a.TestJavaMain\nimport a.TextUtil\nimport a.Gone\ndef s = \"\"\"\n    mainClass.set(\"${TestJavaMain.class.name}\")\n    classpath(\"${TextUtil.escapeString(x)}\")\n    echo \"\\$Gone\"\n\"\"\"\n";
        assert_eq!(rules(src), vec![("UnusedImport", 3)]);
        // a `$` in a single-quoted string is text
        let src = "import a.X\ndef s = '''\n\"${X}\"\n'''\n";
        assert_eq!(rules(src), vec![("UnusedImport", 1)]);
    }

    #[test]
    fn comments_count_whole_words_only() {
        let src = "import a.List\nimport a.Linked\n// Listing, {@link Linked#x}\nclass X {}\n";
        assert_eq!(rules(src), vec![("UnusedImport", 1)]);
    }

    #[test]
    fn unused_import_is_linear_in_comment_text() {
        // a substring search of all comment text per import took over 10 s on this (5 MB) in a
        // release build; one word set takes under 1 s
        let imports: String = (0..30_000).map(|i| format!("import a.C{i}\n")).collect();
        let src = format!(
            "{imports}{}",
            "// lorem ipsum dolor sit amet\n".repeat(160_000)
        );
        let t = std::time::Instant::now();
        assert_eq!(lint(&src).expect("parses").len(), 30_000);
        assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    }

    #[test]
    fn aliased_import_checks_alias() {
        assert_eq!(rules("import a.b.Long as L\nL x = null\n"), vec![]);
        assert_eq!(
            rules("import a.b.Long as L\nLong x = null\n"),
            vec![("UnusedImport", 1)]
        );
    }

    #[test]
    fn gstrings() {
        let src = "def a = \"plain\"\ndef b = \"hi ${a}\"\ndef c = 'single'\ndef d = \"it's\"\n";
        assert_eq!(rules(src), vec![("UnnecessaryGString", 1)]);
    }

    #[test]
    fn semicolons() {
        assert_eq!(
            rules("def a = 1;\ndef b = 2; def c = 3\n"),
            vec![("UnnecessarySemicolon", 1)]
        );
        assert_eq!(rules("for (int i = 0; i < 3; i++) { println i }\n"), vec![]);
    }

    #[test]
    fn empty_catch_blocks() {
        let src = "try { x() } catch (Exception e) { }\ntry { x() } catch (Exception ignored) { }\ntry { x() } catch (Exception e) { log(e) }\n";
        assert_eq!(rules(src), vec![("EmptyCatchBlock", 1)]);
    }

    #[test]
    fn empty_blocks() {
        let src = "if (a) { }\nif (a) { x() } else { }\ntry { } finally { }\nwhile (a) { }\nfor (i in xs) { }\nif (a) {\n  // intentionally nothing\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyIfStatement", 1),
                ("EmptyElseBlock", 2),
                ("EmptyTryBlock", 3),
                ("EmptyFinallyBlock", 3),
                ("EmptyWhileStatement", 4),
                ("EmptyForStatement", 5),
            ]
        );
    }

    #[test]
    fn conditionals() {
        let src = "if (x = 1) { f() }\nwhile ((y = next())) { f() }\nif (x == 1) { f() }\nif (true) { f() }\nif (x == x) { f() }\nif (f() == f()) { g() }\n";
        assert_eq!(
            rules(src),
            vec![
                ("AssignmentInConditional", 1),
                ("ConstantIfExpression", 4),
                ("ComparisonWithSelf", 5),
            ]
        );
    }

    #[test]
    fn duplicate_keys_and_cases() {
        let src = "def m = [a: 1, 'a': 2, b: 3, 1: 'x', '1': 'y', \"${k}\": 4, \"${k}\": 5]\nswitch (x) {\n  case 1: f(); break\n  case 2: g(); break\n  case 1: h(); break\n  case 'a': f(); break\n  case '1': f(); break\n  case \"a\": f(); break\n  case '''a''': f(); break\n}\n";
        assert_eq!(
            rules(src),
            vec![
                ("DuplicateMapKey", 1),
                ("DuplicateCaseStatement", 5),
                ("DuplicateCaseStatement", 8),
                ("UnnecessaryGString", 8),
                ("DuplicateCaseStatement", 9)
            ]
        );
    }

    #[test]
    fn every_rule_fires_and_every_emitted_rule_is_registered() {
        let src = "import a.B\nimport a.B\nimport c.Unused\ndef s = \"x\";\ntry { f() } catch (e) { }\nif (true) { }\nif (a = 1) { f() } else { }\ntry { } finally { }\nwhile (a) { }\nfor (i in xs) { }\ndef m = [a: 1, a: 2]\nswitch (x) { case 1: f(); break\n case 1: g() }\nif (x == x) { f() }\nB.go()\n";
        let mut emitted: HashSet<&str> =
            lint(src).expect("parses").iter().map(|v| v.rule).collect();
        let jenkins = "pipeline {\n agent foo\n agent none\n def x = 1\n bogus { }\n options { parameters([]) }\n stages {\n  stage(\"${x}\") {\n   when { onBranch 'x' }\n   steps { if (x) { } }\n  }\n  stage('a') { }\n  stage('a') { steps { echo 1 } }\n }\n}\n";
        let gradle =
            "repositories { jcenter() }\ndependencies { compile 'a:b:+' }\ntask t << { }\n";
        let spock = "class S extends Specification {\n def f() {\n  when:\n  f()\n  then: 'x'\n  where:\n  a | b\n  2 | 3 | 4\n }\n def g() {\n  then:\n  x\n }\n}\n";
        let more = "package p\nimport p.Q\nimport java.util.Date\nclass K {\n private int pf\n private void m() { }\n static { }\n { }\n boolean equals(Object o) { true }\n boolean equals(String s) { true }\n int hashcode() { 1 }\n String toString() { null }\n}\ndef g() {\n def u = 1\n return 1\n f()\n}\ntry { f() } finally { return 1; throw new E(); assert x }\nswitch (x) { }\ndef s = [1, 1] as Set\ndef t = true ? 1 : 2\nassert true\nif (1 == 2) { f() }\nif (x != null || x.a) { f() }\nif (x % 2 == 1) { f() }\nl.removeAll(l)\nthrow RuntimeException()\nvoid h() { try { f() } catch (e) { new BadException(e) } }\nDate d = new Q()\n";
        for (name, src) in [
            ("A.groovy", more),
            ("B.groovy", "// groovyfmt-ignore DeadCode\nprintln 1\n"),
            ("Jenkinsfile", jenkins),
            ("build.gradle", gradle),
            ("S.groovy", spock),
        ] {
            emitted.extend(
                lint_file(Path::new(name), src)
                    .expect("parses")
                    .iter()
                    .map(|v| v.rule),
            );
        }
        let all: HashSet<&str> = RULES.iter().map(|r| r.name).collect();
        assert_eq!(emitted, all);
    }

    #[test]
    fn exemptions_do_not_fire() {
        // nextflow NextflowDSLImpl: `if( (varX=isVariableX(stm.expression)) )`
        let src = "try { f() } catch (Exception ignore) { }\nif (x += 1) { f() }\nif (\"${x}\") { f() }\nenum E { A, B;\n  int v }\nif ((m = s.find(p))) { f() }\n";
        assert_eq!(rules(src), vec![]);
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        // `é` is two bytes; the `;` is the 12th character but the 13th byte
        let v = lint("def s = 'é';\n").expect("parses");
        assert_eq!(
            (v[0].rule, v[0].line, v[0].col, v[0].end_line, v[0].end_col),
            ("UnnecessarySemicolon", 1, 12, 1, 13)
        );
        let v = lint("try { f() } catch (é) {\n}\n").expect("parses");
        assert_eq!(
            (v[0].rule, v[0].line, v[0].col, v[0].end_line, v[0].end_col),
            ("EmptyCatchBlock", 1, 13, 2, 2)
        );
    }

    #[test]
    fn empty_loop_body_semicolon_is_never_reported() {
        // The `;` is the loop's whole body: removing it would make `println` the body.
        assert_eq!(lint("while (it.next());\nprintln 'x'\n"), Ok(vec![]));
    }

    #[test]
    fn semicolon_before_a_closure_line_is_kept() {
        // GPars AsyncInvocationWithTimeoutTest: without the `;` the closure becomes an
        // argument of `new DataflowVariable()`
        let src = "final DataflowVariable result = new DataflowVariable();\n{-> result << 20 }.callTimeoutAsync(timeout);\nsleep 3000\n";
        assert_eq!(rules(src), vec![("UnnecessarySemicolon", 2)]);
        // a comment between doesn't matter
        assert_eq!(rules("f();\n// why\n{ -> g() }()\n"), vec![]);
        // bigtop TestBlockRecovery: without the `;`, `x[0]` then `++n` reads as `x[0]++`
        assert_eq!(
            rules("def x = f()[0];\n++n;\nm()\ny();\n--n\n"),
            vec![("UnnecessarySemicolon", 2)]
        );
    }

    #[test]
    fn annotated_imports_are_never_unused() {
        // a Jenkins shared library or a Grape dependency is loaded by the annotation
        let src =
            "@Library('lib@master')\nimport org.foo.Zot\n@Grab('a:b:1') import c.D\nimport e.F\n";
        let v = lint(src).expect("parses");
        assert_eq!(
            v.iter().map(|v| (v.rule, v.line)).collect::<Vec<_>>(),
            vec![("UnusedImport", 4)]
        );
        let dup = lint("@Grab('a:b:1') import c.D\n@Grab('a:b:1') import c.D\nD.go()\n").unwrap();
        assert_eq!(dup.len(), 1);
        assert_eq!((dup[0].rule, dup[0].fix.clone()), ("DuplicateImport", None));
    }

    #[test]
    fn gorm_where_queries_compare_a_property_with_a_variable() {
        let cfg = |src: &str| -> Vec<(&'static str, usize)> {
            rules(src)
                .into_iter()
                .filter(|(r, _)| *r == "ComparisonWithSelf")
                .collect()
        };
        // grails-core WhereQueryVariableScopeService, EmbeddedWhereClauseSpec, WhereMethodSpec
        let src = "def f(String title) {\n    Book.where {\n        title == title\n    }.list()\n}\ninterface S {\n    @Where({ contexts { n == n } })\n    List f(String n)\n}\nPerson.where { age > age && pets { age > age } }\nwhere { a == a }\n";
        assert_eq!(cfg(src), vec![]);
        // other closures still count
        let src =
            "xs.findAll { title == title }\ndef where(x) { x == x }\nBook.where(title == title)\n";
        assert_eq!(
            cfg(src),
            vec![
                ("ComparisonWithSelf", 1),
                ("ComparisonWithSelf", 2),
                ("ComparisonWithSelf", 3)
            ]
        );
    }

    #[test]
    fn explain_examples_show_their_rule_and_fixable_matches_the_fixes() {
        for r in RULES {
            // the domain rules look at the file name
            let path = match &r.name[..r.name.len().min(6)] {
                "Jenkin" => "Jenkinsfile",
                "Gradle" => "build.gradle",
                _ => "MathSpec.groovy",
            };
            let fired = |src: &str| {
                lint_file(Path::new(path), src)
                    .expect(src)
                    .into_iter()
                    .filter(|v| v.rule == r.name)
                    .collect::<Vec<_>>()
            };
            let bad = fired(r.bad);
            assert!(!bad.is_empty(), "{}: bad example doesn't fire", r.name);
            assert!(fired(r.good).is_empty(), "{}: good example fires", r.name);
            assert_eq!(bad.iter().all(|v| v.fix.is_some()), r.fixable, "{}", r.name);
            assert!(!r.why.is_empty(), "{}", r.name);
        }
    }

    #[test]
    fn unparseable_is_not_linted() {
        assert_eq!(lint("def x = {\n"), Err("parse error"));
    }
}
