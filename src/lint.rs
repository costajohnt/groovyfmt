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
        bad: "while (running) {\n}",
        good: "while (running) {\n    poll()\n}",
        fixable: false,
    },
    Rule {
        name: "EmptyForStatement",
        default: true,
        description: "for loop with an empty block",
        why: "A `for` loop with an empty block does nothing. Not flagged: a for-in loop over \
              anything but a literal, which may be draining an iterator, and a `for (;;)` loop \
              whose condition calls or assigns, or whose update calls or reads a field \
              (`n = n.next`), as for `while`.",
        bad: "for (i in 0..<n) {\n}",
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
        fixable: true,
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
    Rule {
        name: "BigDecimalInstantiation",
        default: false,
        description: "new BigDecimal(1.1): a BigDecimal made from a double literal, which is inexact (off by default: on real code it found only tests of BigDecimal itself, with values a double holds exactly)",
        why: "A decimal literal passed to the BigDecimal constructor goes through a double first, so `new BigDecimal(0.1)` is 0.1000000000000000055511151231257827. Pass the number as a String to get exactly the digits written.",
        bad: "def rate = new BigDecimal(0.1)",
        good: "def rate = new BigDecimal('0.1')",
        fixable: false,
    },
    Rule {
        name: "BitwiseOperatorInConditional",
        default: false,
        description: "`|` or `&` in an if, while, ternary or elvis condition (off by default: on real code nearly every one tests bit flags on purpose)",
        why: "`|` and `&` evaluate both sides and are easily typed for `||` and `&&`. If the bitwise operation is meant, a variable holding its result says so.",
        bad: "if (ready | retry) {\n    run()\n}",
        good: "if (ready || retry) {\n    run()\n}",
        fixable: false,
    },
    Rule {
        name: "DoubleNegative",
        default: false,
        description: "`!!x` or `!(!x)` (off by default: `!!x` is a common way to turn a value into a boolean)",
        why: "A double negative is hard to read. Where it turns a value into a boolean, `x as boolean` says so.",
        bad: "def found = !!result",
        good: "def found = result as boolean",
        fixable: false,
    },
    Rule {
        name: "EmptySynchronizedStatement",
        default: false,
        description: "a synchronized block with nothing in it (off by default: real code gave nothing to measure it on)",
        why: "A `synchronized` block with nothing in it takes and releases a lock and does nothing else: an unfinished edit, or a misguided attempt at a memory barrier.",
        bad: "synchronized (lock) {\n}",
        good: "synchronized (lock) {\n    count++\n}",
        fixable: false,
    },
    Rule {
        name: "ForLoopShouldBeWhileLoop",
        default: false,
        description: "for (; condition;) with only a condition (off by default: a style preference)",
        why: "A `for` loop without an initializer or an update is a `while` loop written in a harder way.",
        bad: "for (; it.hasNext();) {\n    process(it.next())\n}",
        good: "while (it.hasNext()) {\n    process(it.next())\n}",
        fixable: false,
    },
    Rule {
        name: "IntegerGetInteger",
        default: false,
        description: "Integer.getInteger(name), which reads a system property (off by default: on real code it found mostly deliberate reads of system properties)",
        why: "`Integer.getInteger(s)` doesn't parse `s`: it reads the system property named `s`. That reads like parsing, and it is sometimes called by mistake for `Integer.parseInt(s)`.",
        bad: "def port = Integer.getInteger(text)",
        good: "def port = Integer.parseInt(text)",
        fixable: false,
    },
    Rule {
        name: "RandomDoubleCoercedToZero",
        default: false,
        description: "Math.random() cast to int or long, or assigned to an int or long field or returned from a method returning one (off by default: real code gave nothing to measure it on)",
        why: "`Math.random()` is a double from 0 up to 1, so as an int or a long it is always 0.",
        bad: "int pick = (int) Math.random()",
        good: "int pick = (int) (Math.random() * size)",
        fixable: false,
    },
    Rule {
        name: "UnnecessaryInstanceOfCheck",
        default: true,
        description: "`!x instanceof T`, which tests the boolean `!x`",
        why: "`!` binds tighter than `instanceof`, so `!x instanceof String` asks whether a boolean is a String: always false (always true for Boolean).",
        bad: "if (!value instanceof String) {\n    convert(value)\n}",
        good: "if (!(value instanceof String)) {\n    convert(value)\n}",
        fixable: false,
    },
    Rule {
        name: "UnnecessaryModOne",
        default: false,
        description: "`x % 1`, which is always 0 for a whole number (off by default: on real code `x % 1 != 0` tested decimals for a fractional part, which works)",
        why: "Any whole number modulo 1 is 0, so the check meant is usually `x % 2` or `x & 1`. On a decimal, `x % 1` is its fractional part, which is a correct use.",
        bad: "if (count % 1 == 0) {\n    flush()\n}",
        good: "if (count % 2 == 0) {\n    flush()\n}",
        fixable: false,
    },
    Rule {
        name: "ClassNameSameAsFilename",
        default: false,
        description: "a file holding only one class, interface or enum named differently than the file (off by default: a naming convention)",
        why: "With one class in a file, naming the file after it is how readers and tools find it.",
        bad: "// MathSpec.groovy\nclass MathHelpers {\n}",
        good: "// MathSpec.groovy\nclass MathSpec {\n}",
        fixable: false,
    },
    Rule {
        name: "Groovy24EmptyElement",
        default: true,
        description: "an empty argument, list element or map entry (`foo(a,,b)`, `[a, , b]`, `[a: 1,, b: 2]`), which Groovy 2.4 drops and Groovy 3 and later reject",
        why: "Groovy 2.4 (Hubitat, SmartThings) ignores the extra comma, so it does nothing there, and Groovy 3 and later refuse to compile the file. Removing it changes nothing on 2.4 and makes the code portable.",
        bad: "def modes = [\"off\", \"cool\", , \"heat\"]\nsetModes(modes,, true)",
        good: "def modes = [\"off\", \"cool\", \"heat\"]\nsetModes(modes, true)",
        fixable: true,
    },
    // Unused code (src/lint/unused.rs)
    Rule {
        name: "UnusedPrivateField",
        default: true,
        description: "a private field whose name appears nowhere else in the file, nor in another file of the project that names its class (not one with an annotation, serialVersionUID, or in a class whose annotation reads fields)",
        why: "A private field nothing reads is dead state: left over from a refactoring, or a sign that the code meant to use it doesn't. A private member used only by a DSL script run with the class as its delegate (a `DelegatingScript`, a builder) looks unused here; mark it `@SuppressWarnings('UnusedPrivateField')` (or the class, for all of them), since removing it breaks the script.",
        bad: "class Client {\n    private int retries = 3\n    def get(url) { url.toURL().text }\n}",
        good: "class Client {\n    def get(url) { url.toURL().text }\n}",
        fixable: false,
    },
    Rule {
        name: "UnusedPrivateMethod",
        default: true,
        description: "a private method whose name appears nowhere else in the file (for getX/isX/setX, nor the property x), nor in another file of the project that names its class",
        why: "A private method nothing calls is dead code that still has to be read and maintained. A private member used only by a DSL script run with the class as its delegate (a `DelegatingScript`, a builder) looks unused here; mark it `@SuppressWarnings('UnusedPrivateMethod')` (or the class, for all of them), since removing it breaks the script.",
        bad: "class Client {\n    def get(url) { url.toURL().text }\n    private String trimmed(s) { s.trim() }\n}",
        good: "class Client {\n    def get(url) { url.toURL().text }\n}",
        fixable: false,
    },
    Rule {
        name: "UnusedVariable",
        default: true,
        description: "a local variable whose name never appears after its declaration in its block (not a script's top-level variable, `_`, `ignored`, Spock's `E e = thrown()`, an annotated one, or a `def (a, b)` tuple)",
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
    Rule {
        name: "JenkinsParallelNesting",
        default: true,
        description: "parallel or matrix inside a stage that is already inside parallel or matrix, or agent or tools on a stage holding parallel or matrix",
        why: "Jenkins runs one level of parallel stages: a parallel or matrix inside another one is rejected, and so are agent and tools on the stage that holds them, since that stage runs nothing itself. Source: the Declarative Pipeline validator (ModelValidatorImpl, messages NoNestedWithinNestedStages, AgentInNestedStages, ToolsInNestedStages).",
        bad: "pipeline {\n    agent none\n    stages {\n        stage('Test') {\n            agent any\n            parallel {\n                stage('Unit') {\n                    steps {\n                        sh 'make test'\n                    }\n                }\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent none\n    stages {\n        stage('Test') {\n            parallel {\n                stage('Unit') {\n                    agent any\n                    steps {\n                        sh 'make test'\n                    }\n                }\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsWhenNesting",
        default: true,
        description: "allOf or anyOf with no condition, not without exactly one, arguments to them, or a block of conditions under another condition",
        why: "Only allOf, anyOf and not nest conditions: allOf and anyOf need at least one, not exactly one, and none of them takes arguments. Any other condition given a block is rejected when the build starts. Source: the Declarative Pipeline validator (ModelValidatorImpl, messages NestedWhenWithoutChildren, NestedWhenWrongChildrenCount, NestedWhenNoArgs, NoNestedWhenAllowed).",
        bad: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            when {\n                not {\n                    branch 'dev'\n                    branch 'test'\n                }\n            }\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            when {\n                not {\n                    anyOf {\n                        branch 'dev'\n                        branch 'test'\n                    }\n                }\n            }\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsMatrixAxes",
        default: true,
        description: "a matrix without axes, an axis without a valid name or values, a repeated axis name or value, an empty excludes or exclude",
        why: "A matrix needs `axes`, each axis a `name` that is a valid variable name and `values` that are plain strings, with no name or value given twice; `excludes` and each `exclude` must not be empty, and an exclude axis takes `values` or `notValues`, not both. Jenkins rejects anything else when the build starts. Source: the Declarative Pipeline parser and validator (ModelParser, ModelValidatorImpl, messages RequiredSection, NoAxes, DuplicateAxisName, DuplicateAxisValue, NoExcludes, MatrixExcludeAxisValuesOrNotValues).",
        bad: "pipeline {\n    agent none\n    stages {\n        stage('Test') {\n            matrix {\n                axes {\n                    axis {\n                        name 'OS'\n                        values 'linux', 'mac', 'linux'\n                    }\n                }\n                stages {\n                    stage('Unit') {\n                        steps {\n                            sh 'make test'\n                        }\n                    }\n                }\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent none\n    stages {\n        stage('Test') {\n            matrix {\n                axes {\n                    axis {\n                        name 'OS'\n                        values 'linux', 'mac'\n                    }\n                }\n                stages {\n                    stage('Unit') {\n                        steps {\n                            sh 'make test'\n                        }\n                    }\n                }\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsInputSection",
        default: true,
        description: "a stage input without message, with a field other than message, id, ok, submitter, submitterParameter, parameters, or a field twice",
        why: "A stage's `input` needs a `message`, takes only the fields message, id, ok, submitter, submitterParameter and parameters, each once with one value; Jenkins rejects anything else when the build starts. Source: the Declarative Pipeline parser and validator (ModelParser.parseInput, messages InvalidInputField, MultipleOfSection, MissingInputMessage).",
        bad: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            input {\n                text 'Deploy to production?'\n            }\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    stages {\n        stage('Deploy') {\n            input {\n                message 'Deploy to production?'\n            }\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsEnvironmentEntry",
        default: true,
        description: "an environment line that isn't KEY = value, a key that isn't a valid name or is set twice, or a value that isn't a string, literal or method call",
        why: "`environment` takes only `KEY = value` lines with unique keys made of letters, digits and `_`. A value must be a quoted string, a literal, a method call such as credentials('id'), or quoted strings joined with `+`; a bare `env.X` or `params.X` is rejected when the build starts (write \"${params.X}\"). Source: the Declarative Pipeline parser and validator (ModelParser.parseEnvironment, messages InvalidEnvironmentValue, InvalidEnvironmentConcatValue, InvalidEnvironmentOperation, DuplicateEnvVar, InvalidIdentifierInEnv).",
        bad: "pipeline {\n    agent any\n    environment {\n        TARGET = params.TARGET\n    }\n    stages {\n        stage('Deploy') {\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    environment {\n        TARGET = \"${params.TARGET}\"\n    }\n    stages {\n        stage('Deploy') {\n            steps {\n                sh 'make deploy'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsDuplicateOption",
        default: true,
        description: "the same option or trigger twice in one options or triggers block",
        why: "An option or trigger given twice in one block is rejected when the build starts, so the second one never silently wins. Source: the Declarative Pipeline validator (ModelValidatorImpl, messages DuplicateOptionName, DuplicateTriggerName).",
        bad: "pipeline {\n    agent any\n    options {\n        timeout(time: 1, unit: 'HOURS')\n        timeout(time: 2, unit: 'HOURS')\n    }\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    options {\n        timeout(time: 2, unit: 'HOURS')\n    }\n    stages {\n        stage('Build') {\n            steps {\n                sh 'make'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsToolVersion",
        default: true,
        description: "a tools entry that isn't a tool type with exactly one installation name",
        why: "Each line of `tools` names a tool type and one installation, as in maven 'maven-3.9'; a bare type or a second argument is rejected when the build starts. Source: the Declarative Pipeline parser (ModelParser.parseTools, messages ExpectedTool, NoArgForTool, TooManyArgsForTool).",
        bad: "pipeline {\n    agent any\n    tools {\n        maven\n    }\n    stages {\n        stage('Build') {\n            steps {\n                sh 'mvn package'\n            }\n        }\n    }\n}",
        good: "pipeline {\n    agent any\n    tools {\n        maven 'maven-3.9'\n    }\n    stages {\n        stage('Build') {\n            steps {\n                sh 'mvn package'\n            }\n        }\n    }\n}",
        fixable: false,
    },
    // Gradle build scripts (src/lint/gradle.rs)
    Rule {
        name: "GradleRemovedConfiguration",
        default: true,
        description: "compile, runtime, testCompile or testRuntime dependencies (removed in Gradle 7.0), unless the Gradle wrapper pins an older Gradle",
        why: "`compile`, `runtime`, `testCompile` and `testRuntime` were removed in Gradle 7.0; the build fails on a current Gradle. `--fix` renames `testCompile` to testImplementation, `runtime` to runtimeOnly and `testRuntime` to testRuntimeOnly. `compile` becomes implementation for most builds but api for a dependency a library's own API exposes, so `--fix` replaces it only with `gradle_compile_replacement = \"implementation\"` (or `\"api\"`) in the config's `[lint]` table. The new configurations come from the Java plugins, so a rename is fixed only in a file that applies one (java, java-library, groovy, scala, application, war, java-gradle-plugin or Kotlin JVM, with `apply plugin:` or in `plugins { }`); a file whose plugins come from elsewhere (a parent's allprojects, a convention plugin) is only reported, and the message says so. `api` comes only with the java-library plugin, so `compile` becomes api only in a file that applies it. Source: the Gradle upgrade guide for 6.x to 7.0, and the Java Library plugin chapter.",
        bad: "plugins {\n    id 'java'\n}\ndependencies {\n    testCompile 'junit:junit:4.13.2'\n}",
        good: "plugins {\n    id 'java'\n}\ndependencies {\n    testImplementation 'junit:junit:4.13.2'\n}",
        fixable: true,
    },
    Rule {
        name: "GradleJcenter",
        default: true,
        description: "the jcenter() repository (removed in Gradle 9.0)",
        why: "JCenter is read-only and `jcenter()` was removed in Gradle 9.0. `--fix` turns a plain `jcenter()` line in a repositories block into mavenCentral(), or removes it when the block already has mavenCentral(); `jcenter { ... }` is left to you. A dependency published only to JCenter still needs another home. Source: the Gradle upgrade guides for 6.x to 7.0 and 8.x to 9.0.",
        bad: "repositories {\n    jcenter()\n}",
        good: "repositories {\n    mavenCentral()\n}",
        fixable: true,
    },
    Rule {
        name: "GradleLeftShiftTask",
        default: true,
        description: "task name << { } (removed in Gradle 5.0)",
        why: "`task name << { }` was removed in Gradle 5.0; the build fails on a current Gradle. `--fix` writes `task name { doLast { ... } }`, keeping the action as written (`task 'name' << { }` is left to you). Source: the Gradle upgrade guide for 4.x to 5.0.",
        bad: "task hello << {\n    println 'hello'\n}",
        good: "task hello { doLast {\n    println 'hello'\n} }",
        fixable: true,
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
    Rule {
        name: "GradleRemovedApi",
        default: true,
        description: "a removed core plugin (maven, findbugs, osgi, ...), the uploadArchives task, archive task properties removed in 8.0, or configurations.compile and the like, unless the Gradle wrapper pins an older Gradle",
        why: "These were removed from Gradle: the `maven` plugin and `uploadArchives` in 7.0 (use maven-publish), findbugs, jdepend, osgi and the announce plugins in 6.0, the `compile`, `runtime`, `testCompile` and `testRuntime` configurations in 7.0, and an archive task's `archiveName`, `baseName`, `classifier`, `destinationDir`, `appendix` and `extension` in 8.0 (use archiveFileName, archiveBaseName, archiveClassifier, destinationDirectory and so on). The build fails on a Gradle without them. Source: the Gradle upgrade guides for 5.x to 6.0, 6.x to 7.0 and 7.x to 8.0.",
        bad: "apply plugin: 'java'\n\njar {\n    classifier = 'all'\n}",
        good: "apply plugin: 'java'\n\njar {\n    archiveClassifier = 'all'\n}",
        fixable: false,
    },
    Rule {
        name: "GradleSpaceAssignment",
        default: true,
        description: "`description 'x'`, `group 'x'` or `version 'x'` setting the project's property (at the top level or in allprojects, subprojects or project(':x')), or `description 'x'` or `group 'x'` in a task's block: Groovy DSL space assignment, deprecated in Gradle 8.12, unless the Gradle wrapper pins an older Gradle",
        why: "Gradle 8.12 deprecated setting a property as `name value`, a method Gradle generates for each property, and Gradle 10 removes it: write `name = value`. Real methods keep working (`apply plugin: 'java'`, `id 'x' version '1'`, `implementation 'a:b:1'`), so only properties known to have no method of the name are reported, where the script's delegate is known: the project's `description`, `group` and `version`, and a task's `description` and `group`. `--fix` writes `name = value`. Source: the Gradle upgrade guide for 8.11 to 8.12 (\"Groovy space assignment syntax\").",
        bad: "group 'org.example'\nversion '1.0'\n\ntask docs {\n    description 'Builds the docs'\n}",
        good: "group = 'org.example'\nversion = '1.0'\n\ntask docs {\n    description = 'Builds the docs'\n}",
        fixable: true,
    },
    Rule {
        name: "GradleDependencyTuple",
        default: false,
        description: "a dependency in map notation, `implementation group: 'a', name: 'b', version: '1'`, with only group, name and version as plain strings (off by default: a style choice before Gradle 9.1)",
        why: "The string `'a:b:1'` says the same in one place, and Gradle 9.1 deprecated the map (multi-string) notation, to be removed in Gradle 10. Not reported with other keys (`classifier`, `ext`, `configuration`, ...) or a value that isn't a plain string. `--fix` writes the string. Source: the Gradle upgrade guide for 9.0 to 9.1 (\"Deprecation of multi-string dependency notation\").",
        bad: "dependencies {\n    implementation group: 'org.slf4j', name: 'slf4j-api', version: '2.0.9'\n}",
        good: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.0.9'\n}",
        fixable: true,
    },
    Rule {
        name: "GradleDependencyParentheses",
        default: false,
        description: "a dependency in parentheses with no closure after it, `implementation('a:b:1')` (off by default: a style choice)",
        why: "In the Groovy DSL `implementation 'a:b:1'` is the same call, and most build scripts write it that way. Not reported with a closure after it, over several lines, or when an argument would read differently without the parentheses. `--fix` removes them.",
        bad: "dependencies {\n    implementation('org.slf4j:slf4j-api:2.0.9')\n}",
        good: "dependencies {\n    implementation 'org.slf4j:slf4j-api:2.0.9'\n}",
        fixable: true,
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
    Rule {
        name: "SpockUnassertedCondition",
        default: true,
        description: "a comparison or a predicate call (xs.contains(x), s.startsWith(p), m.isEmpty(), ...) in a then: or expect: block nested in an if, loop, switch, try or a .each { } (eachWithIndex, reverseEach, forEach, times) closure, which Spock never asserts",
        why: "Spock turns only the top-level expression statements of a then: or expect: block into conditions. A comparison, or a predicate call such as `xs.contains(x)` or `!m.isEmpty()`, inside `if`/`else`, a loop, a `switch` case, `try` or a closure passed to `each` is evaluated and thrown away, so the check can never fail. Write `assert` in front of it, or check a collection with `verifyEach` or `verifyAll { }` (Spock asserts conditions nested in `with { }`, `verifyAll { }`, `verifyEach` and `@Verify` helpers, so those aren't reported). Checked against Spock 2.4.",
        bad: "class ResultSpec extends Specification {\n    def 'parses'() {\n        when:\n        def r = parse(input)\n\n        then:\n        if (valid) {\n            r.value == 42\n        }\n        r.items.each { it.size() > 0 }\n    }\n}",
        good: "class ResultSpec extends Specification {\n    def 'parses'() {\n        when:\n        def r = parse(input)\n\n        then:\n        if (valid) {\n            assert r.value == 42\n        }\n        r.items.every { it.size() > 0 }\n    }\n}",
        fixable: false,
    },
];

mod basic;
mod elsewhere;
mod gradle;
mod imports;
mod jenkins;
mod spock;
mod suppress;
mod unused;

pub use elsewhere::Projects;
pub(crate) use gradle::{same_assignment, same_call};

#[derive(Debug, PartialEq)]
pub struct Violation {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based, in characters
    /// Where the flagged node ends: the line, and the column just after its last character.
    pub end_line: usize,
    pub end_col: usize,
    pub rule: &'static str,
    pub message: String,
    /// What `--fix` does to fix it, for the rules it fixes: delete the `;`, or the import's
    /// whole lines when nothing else is on them; replace a removed Gradle configuration's
    /// name; rewrite a Gradle statement.
    pub fix: Option<Edit>,
    /// The bytes of the flagged code, so `--fix` can tell a report that goes with a deleted
    /// line (an unused import that is also unnecessary) from one that changed.
    pub span: std::ops::Range<usize>,
    /// UnusedPrivateField and UnusedPrivateMethod: the type declaring the member (`None`
    /// at a script's top level), which another file must name to use it
    pub owner: Option<String>,
}

/// `--fix`'s change for one violation: the bytes `range` become `with` (empty: deleted).
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub range: std::ops::Range<usize>,
    pub with: String,
}

pub fn lint(src: &str) -> Result<Vec<Violation>, &'static str> {
    lint_file(Path::new(""), src)
}

/// Fixes that depend on the config: with `gradle_compile_replacement` set (`compile`),
/// GradleRemovedConfiguration on a `compile` dependency replaces the name with it.
pub fn opt_in_fixes(vs: &mut [Violation], src: &str, compile: Option<&str>) {
    if let Some(with) = compile {
        gradle::opt_in_compile(vs, src, with);
    }
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
            ";" => unnecessary_semicolon(src, n, nodes.get(i + 1..).unwrap_or(&[]), &mut v),
            "catch_clause" => empty_catch(src, n, &mut v),
            "if_statement" => if_statement(n, &mut v),
            "while_statement" if !condition_works(n) => {
                empty_body(
                    n,
                    "body",
                    "EmptyWhileStatement",
                    "The while statement is empty",
                    &mut v,
                );
            }
            // a classic `for` whose condition does work, or whose update walks something
            // (`for (; it.hasNext(); it.next())`, `n = n.next`), as for `while`; a for-in loop over anything
            // but a literal may be draining an iterator (`for (d in data) {}`)
            "for_statement" if !condition_works(n) && !update_works(n) => empty_body(
                n,
                "body",
                "EmptyForStatement",
                "The for statement is empty",
                &mut v,
            ),
            "for_in_statement" if iterates_literal(n) => empty_body(
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
    basic::class_name_same_as_filename(path, src, tree.root_node(), &mut v);
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
        owner: None,
    }
}

/// The lines holding `n`, with the line break after them, when nothing but whitespace and
/// a `;` shares them.
fn own_lines(src: &str, n: Node) -> Option<Edit> {
    let start = src[..n.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let end = src[n.end_byte()..]
        .find('\n')
        .map_or(src.len(), |i| n.end_byte() + i + 1);
    let after = src[n.end_byte()..end].trim();
    (src[start..n.start_byte()].trim().is_empty() && (after.is_empty() || after == ";")).then(
        || Edit {
            range: start..end,
            with: String::new(),
        },
    )
}

/// A `qualified_name` as Groovy reads it, from its identifiers: without the spaces, line
/// breaks and comments the source may have around its dots (`a.b. C` is `a.b.C`, #171).
pub(crate) fn dotted(n: Node, src: &str) -> String {
    let mut c = n.walk();
    let parts: Vec<&str> = n
        .named_children(&mut c)
        .filter(|p| p.kind() == "identifier")
        .map(|p| text(p, src))
        .collect();
    parts.join(".")
}

/// An import's name as Groovy reads it (see `dotted`), its alias, or else its last part: the
/// simple name it brings into the file.
pub(crate) fn import_name<'a>(d: Node, src: &'a str) -> (String, &'a str) {
    let name = d.child_by_field_name("name");
    let symbol = d.child_by_field_name("alias").map_or_else(
        || {
            name.and_then(|n| {
                let mut c = n.walk();
                n.named_children(&mut c)
                    .filter(|p| p.kind() == "identifier")
                    .last()
            })
            .map_or("", |p| text(p, src))
        },
        |a| text(a, src),
    );
    (name.map_or_else(String::new, |n| dotted(n, src)), symbol)
}

/// The property names a static import of an accessor (`getFoo`, by its alias if it has
/// one) is also used under: `foo` and `Foo` (#181).
pub(crate) fn import_property_names(d: Node, src: &str) -> Vec<String> {
    if import_has(d, "static") {
        unused::property_names(import_name(d, src).1)
    } else {
        Vec::new()
    }
}

/// Whether import `d` has the keyword or token `t` (`static`, `.*`).
pub(crate) fn import_has(d: Node, t: &str) -> bool {
    let mut c = d.walk();
    d.children(&mut c).any(|k| !k.is_named() && k.kind() == t)
}

/// UnusedImport + DuplicateImport.
fn imports(src: &str, tree: &Tree, v: &mut Vec<Violation>) {
    let (mut decls, mut used) = (Vec::new(), HashSet::new());
    crate::preorder(tree.root_node(), |n| match n.kind() {
        // An import's own name and alias aren't uses, but its annotations are:
        // `@Library('x') import y` uses the `Library` import (#165)
        "import_declaration" => {
            decls.push(n);
            let mut c = n.walk();
            for a in n.named_children(&mut c) {
                if a.kind().contains("annotation") {
                    crate::preorder(a, |m| {
                        if m.kind().contains("comment") {
                            used.extend(words(text(m, src)));
                            false
                        } else {
                            if m.child_count() == 0 {
                                used.insert(text(m, src));
                            }
                            true
                        }
                    });
                }
            }
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
            // The grammar leaves some interpolations as plain text: `"""` holding
            // `"${Name.x}"`, and `$Café.v` or `$Ä`, as its `$name` is ASCII only (#192). Count
            // the names after each unescaped `$` of the whole string.
            if n.kind() == "string_literal" && !text(n, src).starts_with('\'') {
                dollar_names(text(n, src), &mut used);
            }
            if n.child_count() == 0 {
                used.insert(text(n, src));
            }
            true
        }
    });
    // Hubitat appends an `#include`d library to the file, and the library uses the file's
    // imports, so in such a file none can be removed (#192)
    let include = hubitat_include(src, tree.root_node());
    let mut seen = HashSet::new();
    for d in decls.iter().copied() {
        let (name, symbol) = import_name(d, src);
        let alias = d.child_by_field_name("alias").map(|a| text(a, src));
        let star = import_has(d, ".*");
        // as Groovy reads it, so `import a.b. C` and `import a.b.C` are the same (#171); the
        // annotations as written
        let kw = d.children(&mut d.walk()).find(|k| k.kind() == "import");
        let annotations = kw.map_or("", |k| &src[d.start_byte()..k.start_byte()]);
        let full = format!(
            "{}import {}{name}{}{}",
            annotations
                .split_whitespace()
                .map(|w| format!("{w} "))
                .collect::<String>(),
            if import_has(d, "static") {
                "static "
            } else {
                ""
            },
            if star { ".*" } else { "" },
            alias.map_or_else(String::new, |a| format!(" as {a}")),
        );
        // `@Library('lib') import x` and `@Grab(...) import y` are there for the annotation:
        // never unused, and never removed
        let annotated = text(d, src).starts_with('@');
        let fix = own_lines(src, d).filter(|_| !annotated && !include);
        if !seen.insert(full.clone()) {
            // The last import of a simple name wins, so with another import of the name
            // between the copies, `import p.A` / `import q.A` / `import p.A`, removing the
            // later copy switches the class (#192): leave them all
            let rebinds = !star && rebound(src, &decls, d);
            v.push(Violation {
                fix: fix.filter(|_| !rebinds),
                ..at(d, "DuplicateImport", format!("Duplicate import: {full}"))
            });
            continue;
        }
        if star || annotated {
            continue;
        }
        // `import static a.B.getFoo` also serves the property `foo` (Groovy's
        // StaticImportVisitor looks up getFoo / setFoo for it)
        let property_used = import_property_names(d, src)
            .iter()
            .any(|p| used.contains(p.as_str()));
        if !symbol.is_empty() && !used.contains(symbol) && !property_used {
            v.push(Violation {
                fix,
                ..at(
                    d,
                    "UnusedImport",
                    match alias {
                        Some(a) => {
                            format!("The alias [{a}] of the [{name}] import is never referenced")
                        }
                        None => format!("The [{name}] import is never referenced"),
                    },
                )
            });
        }
    }
}

/// Whether an import other than a copy of `d`, and not a `.*` one, brings in the simple
/// name (or alias) `d` does: then which class the name means hangs on their order.
pub(crate) fn rebound(src: &str, decls: &[Node], d: Node) -> bool {
    let (name, symbol) = import_name(d, src);
    decls.iter().any(|&e| {
        let (n, s) = import_name(e, src);
        !import_has(e, ".*")
            && s == symbol
            && (n != name || import_has(e, "static") != import_has(d, "static"))
    })
}

/// Whether the file has a Hubitat `#include ns.Library` line (the grammar reads it as a
/// comment).
pub(crate) fn hubitat_include(src: &str, root: Node) -> bool {
    let mut found = false;
    if src.contains("#include") {
        crate::preorder(root, |n| {
            found |= n.kind().contains("comment") && text(n, src).starts_with("#include");
            !found
        });
    }
    found
}

/// The identifier-like words in `s`.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

/// A character Java may allow in a name (`isJavaIdentifierPart`): wider than `words`, with
/// combining marks and currency signs, so both splits are counted.
fn java_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || (!c.is_ascii() && !c.is_whitespace() && !c.is_control())
}

/// The names after each unescaped `$` in GString text: `$a.b`, and every word in `${...}`,
/// Unicode ones included (`$Café.v`, #192).
pub(crate) fn dollar_names<'a>(t: &'a str, used: &mut impl Extend<&'a str>) {
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
                        .find(|c: char| !(java_part(c) || c == '.'))
                        .map_or(b.len(), |k| start + k)
                };
                used.extend(words(&t[start..end]));
                used.extend(
                    t[start..end]
                        .split(|c| !java_part(c))
                        .filter(|w| !w.is_empty()),
                );
                i = end;
            }
            _ => i += 1,
        }
    }
}

/// UnnecessaryGString: double-quoted string with nothing to interpolate. Groovy compiles one
/// to the same String constant as its single-quoted form, which the fix writes: the same
/// escapes mean the same in both. Not fixed with a `\u` escape, which Groovy reads before
/// the quotes (`"\u0027"` would end a single-quoted string).
fn unnecessary_gstring(src: &str, n: Node, v: &mut Vec<Violation>) {
    let t = text(n, src);
    if t.starts_with('"') && !has_interpolation(n) && !t.contains('\'') && !t.contains('$') {
        let q = if t.starts_with("\"\"\"") { 3 } else { 1 };
        let fix = (!t.contains("\\u") && t.len() >= 2 * q).then(|| Edit {
            range: n.byte_range(),
            with: format!("{0}{1}{0}", "'".repeat(q), &t[q..t.len() - q]),
        });
        v.push(Violation {
            fix,
            ..at(
                n,
                "UnnecessaryGString",
                format!("The String {t} can be wrapped in single quotes instead of double quotes"),
            )
        });
    }
}

/// UnnecessarySemicolon: a `;` that ends a line (or the file). Not one that is a loop's
/// empty body (`while (it.next());`), or one before a line starting with `{`
/// (`foo();` then `{ -> x }.call()`) or `++` / `--` (`a[0];` then `++n`): either way
/// removing it would change the program, here by making the closure an argument of the
/// statement before, or the `++` a postfix increment of it. Nor one between top-level
/// statements that Groovy 3+ would join into a method declaration (`joins_declaration`).
fn unnecessary_semicolon(src: &str, n: Node, rest: &[Node], v: &mut Vec<Violation>) {
    if n.parent().is_some_and(|p| {
        p.kind().contains("enum")
            || p.kind() == "for_statement"
            || (p.kind() == "while_statement" && p.child_by_field_name("body") == Some(n))
            || (p.kind() == "source_file"
                && statement_beside(n, Node::prev_sibling)
                    .zip(statement_beside(n, Node::next_sibling))
                    .is_some_and(|(a, b)| joins_declaration(src, a, b)))
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
            fix: Some(Edit {
                range: n.byte_range(),
                with: String::new(),
            }),
            ..at(
                n,
                "UnnecessarySemicolon",
                "Semicolons as line endings can be removed safely".into(),
            )
        });
    }
}

/// The statement next to `n` in the direction of `step`, past comments and other `;`s.
fn statement_beside<'t>(n: Node<'t>, step: fn(&Node<'t>) -> Option<Node<'t>>) -> Option<Node<'t>> {
    let mut x = step(&n)?;
    while x.kind().contains("comment") || x.kind() == ";" {
        x = step(&x)?;
    }
    x.is_named().then_some(x)
}

/// Whether Groovy 3 and 4 read the top-level statements `a` and `b`, on separate lines with
/// no `;` between them, as one method declaration (#211): `a` a type (a name whose last part
/// is capitalized: `FOO`, `a.B`, `List<String>`, `String[]`) and `b` a call
/// (`bar()`, `'bar'(x)`), so `FOO` then `bar()` declares `FOO bar()`, an error without a
/// body, and `FOO` then `bar(x) { x }` silently declares a method. Groovy 2 reads two
/// statements. Any call is taken, though only one whose arguments read as parameters
/// (`bar(x)`, `bar(Foo)`, not `bar(1)`) joins.
pub(crate) fn joins_declaration(src: &str, a: Node, b: Node) -> bool {
    if a.kind() != "expression_statement" || b.kind() != "expression_statement" {
        return false;
    }
    let mut t = text(a, src).trim_end();
    while let Some(r) = t.strip_suffix("[]") {
        t = r.trim_end();
    }
    if t.ends_with('>') {
        t = t.split('<').next().unwrap_or(t);
    }
    let ident = |c: char| java_part(c) || c == '$';
    let typed = !matches!(t.split('.').next().map(str::trim), Some("this" | "super"))
        && t.split('.')
            .map(str::trim)
            .all(|p| !p.is_empty() && p.chars().all(ident))
        && t.rsplit('.')
            .next()
            .is_some_and(|p| p.trim().starts_with(char::is_uppercase));
    let call = text(b, src);
    let name = match call.chars().next() {
        Some(q @ ('\'' | '"')) => call[1..].find(q).map_or(call.len(), |i| i + 2),
        _ => call.find(|c| !ident(c)).unwrap_or(call.len()),
    };
    typed
        && name > 0
        && call[name..]
            .trim_start_matches([' ', '\t'])
            .starts_with('(')
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

/// Whether a loop's condition does the work, so an empty body is fine:
/// `while (reader.read() != -1) {}`, `while ((line = next()) != null) {}`, `while (i++ < n) {}`.
fn condition_works(n: Node) -> bool {
    works(n.child_by_field_name("condition"))
}

/// Whether `c` calls, assigns or increments: work a loop may do with an empty body.
fn works(c: Option<Node>) -> bool {
    let mut works = false;
    if let Some(c) = c {
        crate::preorder(c, |x| {
            works |= matches!(
                x.kind(),
                "method_invocation"
                    | "assignment_expression"
                    | "update_expression"
                    | "unary_update_expression"
            );
            !works
        });
    }
    works
}

/// Whether a classic `for`'s update walks something, so an empty body is fine: it calls
/// (`it.next()`), or assigns a value read from somewhere other than locals and literals
/// (`n = n.next`, `n = n[0]`). Counting on locals (`i++`, `i += 1`, `i = i + 1`) is not work:
/// `for (int i = 0; i < n; i += 1) {}` is still empty.
fn update_works(n: Node) -> bool {
    let mut works = false;
    // Groovy 3 takes several updates: `i++, it.next()`.
    for u in n.children_by_field_name("update", &mut n.walk()) {
        crate::preorder(u, |x| {
            works |= x.kind() == "method_invocation"
                || (x.kind() == "assignment_expression"
                    && x.child_by_field_name("right").is_some_and(reads_more));
            !works
        });
    }
    works
}

/// Whether `e` reads anything but locals and literals through arithmetic.
fn reads_more(e: Node) -> bool {
    let mut more = false;
    crate::preorder(e, |x| {
        more |= x.is_named()
            && !matches!(
                x.kind(),
                "identifier"
                    | "number_literal"
                    | "boolean_literal"
                    | "null_literal"
                    | "binary_expression"
                    | "unary_expression"
                    | "parenthesized_expression"
            );
        !more
    });
    more
}

/// Whether a for-in loop iterates a literal (`[1, 2]`, `0..<n`, a map or string literal) that
/// does nothing when iterated. Any other value may be an iterator or a lazy iterable whose
/// iteration is the work (`for (d in data) {}`), so an empty loop over it isn't flagged. A
/// call in a list or map literal does work; one in a range's bounds only sizes the range
/// (`0..<list.size()`).
fn iterates_literal(n: Node) -> bool {
    let Some(mut v) = n.child_by_field_name("value") else {
        return false;
    };
    while v.kind() == "parenthesized_expression" {
        match v.named_child(0) {
            Some(c) => v = c,
            None => return false,
        }
    }
    match v.kind() {
        "range_expression" => true,
        "list_literal" | "map_literal" | "string_literal" => !works(Some(v)),
        _ => false,
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
    fn import_names_are_read_without_spaces_or_comments_around_dots() {
        // #171: the name was the text after the last `.`, ` TimeUnit` here, so a used import
        // was reported, and --fix deleted it
        let forms = [
            "import java.util.concurrent. TimeUnit",
            "import java.util.concurrent .TimeUnit",
            "import java.util.concurrent . TimeUnit",
            "import java.util.concurrent.\tTimeUnit",
            "import java.util.concurrent.\n    TimeUnit",
            "import java.util.concurrent\n    .TimeUnit",
            "import java. util . concurrent. TimeUnit",
            "import java.util.concurrent./* c */TimeUnit",
            "import java.util.concurrent. // c\n TimeUnit",
        ];
        for f in forms {
            let src = format!("{f}\nTimeUnit t = TimeUnit.SECONDS\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
            // the same import twice, however it is spaced
            let src = format!("{f}\nimport java.util.concurrent.TimeUnit\nTimeUnit t\n");
            assert_eq!(
                rules(&src),
                vec![("DuplicateImport", 2 + f.matches('\n').count())],
                "{src:?}"
            );
            // and unused, named as Groovy reads it
            let vs = lint(&format!("{f}\nx()\n")).unwrap();
            assert_eq!(
                vs.iter().map(|v| v.message.as_str()).collect::<Vec<_>>(),
                vec!["The [java.util.concurrent.TimeUnit] import is never referenced"],
                "{f:?}"
            );
        }
        for (import, usage) in [
            (
                "import static java.util.concurrent.TimeUnit. SECONDS",
                "println SECONDS",
            ),
            (
                "import static java.util.concurrent.TimeUnit .SECONDS",
                "println SECONDS",
            ),
            (
                "import static java.util.concurrent.TimeUnit.\n  SECONDS",
                "println SECONDS",
            ),
            ("import static a.U. getSvc", "svc.go()"),
            ("import a.b. C as D", "D d = null"),
            ("import a.b.C as\n D", "D d = null"),
            ("import a.b .*", "x()"),
            ("import static a.B\n.*", "x()"),
        ] {
            let src = format!("{import}\n{usage}\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
        }
        // an alias, star or static import is a duplicate only of the same alias, star or
        // static import, however spaced
        assert_eq!(
            rules("import a.b .*\nimport a.b.*\nimport static a.b.*\nx()\n"),
            vec![("DuplicateImport", 2)]
        );
        assert_eq!(
            rules("import a.b. C as D\nimport a.b.C as D\nimport a.b.C as E\nD d; E e\n"),
            vec![("DuplicateImport", 2)]
        );
        assert_eq!(
            rules("import static a.B. c\nimport static a.B.c\nimport a.B.c\nc()\n"),
            vec![("DuplicateImport", 2)]
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
            // in parentheses: Groovy reads `println /x/` as a division
            let src = format!("import a.N\nimport a.M\nprintln({s})\nM.go()\n");
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
    fn annotation_on_an_import_is_a_use() {
        // #165: JenkinsPipelineUnit's `@Library('x@main') import ...` uses `import ...Library`
        for s in [
            "import a.Gen\n@Gen\nimport b.T\nprintln T.S\n",
            "import a.Gen\n@Gen import b.T\nprintln T.S\n",
            "import a.Gen\n@Gen\nimport static b.T.S\nprintln S\n",
            "import a.Gen\n@a.Gen\nimport b.T\nprintln T.S\n",
            "import a.Lib\n@Lib('x@main') import b.T\nprintln T.S\n",
            "import a.Ann\nimport a.Bar\n@Ann(Bar)\nimport b.T\nprintln T.S\n",
            "import a.Out\nimport a.In\n@Out(@In(Bar.X))\nimport b.T\nimport a.Bar\nprintln T.S\n",
        ] {
            assert_eq!(rules(s), vec![], "{s}");
        }
        // the import's own name and alias still aren't uses of anything
        assert_eq!(
            rules("import a.T\n@Ann\nimport b.T as U\n"),
            vec![("UnusedImport", 1)]
        );
    }

    #[test]
    fn names_used_elsewhere_are_uses() {
        // audited for #165, all already counted: package annotations, annotation arguments,
        // class literals, bounds, throws, instanceof, as, casts, catch, method references
        for (import, usage) in [
            ("import a.Ann", "@Ann\npackage p\nprintln 1"),
            ("import a.Bar", "@Ann(Bar)\nclass X {}"),
            ("import a.Bar", "@Ann(value = Bar.X)\nclass X {}"),
            ("import a.In", "@Out(@In)\nclass X {}"),
            ("import a.Bar", "@Ann(Bar.class)\nclass X {}"),
            ("import a.Bar", "class X<T extends Bar> {}"),
            ("import a.Bar", "def <T extends Bar> void f() {}"),
            ("import a.Bar", "void f() throws Bar {}"),
            ("import a.Bar", "if (x instanceof Bar) {}"),
            ("import a.Bar", "def y = x as Bar"),
            ("import a.Bar", "def y = (Bar) x"),
            ("import a.Bar", "List<Bar> l = []"),
            ("import a.Bar", "try {} catch (Foo | Bar e) {}"),
            ("import a.Bar", "def y = Bar::m"),
            ("import a.Bar", "def f(Bar... b) {}"),
            ("import a.Bar", "class X { void f(@Bar x) {} }"),
            ("import static a.B.c", "c()"),
        ] {
            let src = format!("{import}\n{usage}\n");
            let unused: Vec<_> = rules(&src)
                .into_iter()
                .filter(|r| r.0 == "UnusedImport")
                .collect();
            assert_eq!(unused, vec![], "{src:?}");
        }
    }

    #[test]
    fn unused_aliased_import_names_the_alias() {
        let v = lint("import a.B as C\nimport a.D\n").unwrap();
        let got: Vec<_> = v.iter().map(|v| v.message.as_str()).collect();
        assert_eq!(
            got,
            vec![
                "The alias [C] of the [a.B] import is never referenced",
                "The [a.D] import is never referenced"
            ]
        );
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
        let run = |n: usize| {
            let imports: String = (0..n).map(|i| format!("import a.C{i}\n")).collect();
            let src = format!(
                "{imports}{}",
                "// lorem ipsum dolor sit amet\n".repeat(n * 16 / 3)
            );
            assert_eq!(lint(&src).expect("parses").len(), n);
        };
        // up to 10 MB, the largest file linted
        crate::scaling::assert_scales_up_to(
            "imports against comment text",
            run,
            7_500,
            60_000,
            4,
            8,
        );
    }

    #[test]
    fn static_accessor_import_used_as_a_property() {
        // hoist-core ClusterTask: `traceContextService.capture()` resolves to getTraceContextService()
        let src = "import static a.U.getSvc\nimport static a.U.setOut\nimport static a.U.getGone\nimport a.getThing\nsvc.go()\nout = 1\nthing\n";
        assert_eq!(rules(src), vec![("UnusedImport", 3), ("UnusedImport", 4)]);
    }

    #[test]
    fn static_accessor_import_used_under_each_property_name_groovy_reads() {
        // #181 (Alipsa/matrix MatrixDbUtil): Groovy 2.4.21, 3.0.25 and 4.0.24 read each of
        // these uses through the import; a name whose first two letters are capitals stays
        for (import, used) in [
            ("getSCOPES", "SCOPES"),
            ("getDECIMAL_PRECISION", "DECIMAL_PRECISION"),
            ("isACTIVE", "ACTIVE"),
            ("getHTTPHost", "HTTPHost"),
            ("getURL2", "URL2"),
            ("getUrl", "url"),
            ("getUrl", "Url"),
            ("getQ", "q"),
            ("getQ", "Q"),
            ("isB", "b"),
            ("isB", "B"),
            ("getA1", "a1"),
            ("getA1", "A1"),
            ("get_foo", "_foo"),
            ("get_Bar", "_Bar"),
            ("getaB", "aB"),
            ("get\u{c4}pfel", "\u{e4}pfel"),
            ("get\u{c4}pfel", "\u{c4}pfel"),
            ("get$d", "$d"),
        ] {
            let src = format!("import static a.U.{import}\nprintln {used}\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
            let src =
                format!("import static a.U.{import}\nclass W {{ def go() {{ id({used}) }} }}\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
        }
        for (import, used) in [
            ("setURLX", "URLX"),
            ("setName", "Name"),
            ("setUrlY", "urlY"),
        ] {
            let src = format!("import static a.U.{import}\n{used} = 1\n");
            assert_eq!(rules(&src), vec![], "{src:?}");
        }
        // still unused: no form of the property appears, the import isn't static, or there
        // is no property (`get` alone)
        for src in [
            "import static a.U.getSCOPES\nprintln scopesList\n",
            "import static a.U.getHTTPHost\nprintln host\n",
            "import static a.U.isACTIVE\nprintln inactive\n",
            "import static a.U.setURLX\nURL = 1\n",
            "import a.getSCOPES\nprintln SCOPES\n",
            "import static a.U.get\nprintln x\n",
        ] {
            assert_eq!(rules(src), vec![("UnusedImport", 1)], "{src:?}");
        }
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
        let src = "if (a) { }\nif (a) { x() } else { }\ntry { } finally { }\nwhile (a) { }\nfor (i in 0..2) { }\nif (a) {\n  // intentionally nothing\n}\n";
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
        // a condition that calls, assigns or increments does the loop's work
        let src = "while (reader.read() != -1) { }\nwhile ((line = next()) != null) { }\nwhile (i++ < n) { }\nfor (; it.next() != x; ) { }\nwhile (true) { }\nfor (x in [1, 2]) { }\nfor (int i = 0; i < n; i++) { }\nwhile (a.b) { }\n";
        assert_eq!(
            rules(src),
            vec![
                ("ForLoopShouldBeWhileLoop", 4),
                ("EmptyWhileStatement", 5),
                ("EmptyForStatement", 6),
                ("EmptyForStatement", 7),
                ("EmptyWhileStatement", 8),
            ]
        );
        // a for-in loop over anything but a literal may drain an iterator; a classic `for`
        // whose update calls or assigns walks something
        let src = "for (d in data) { }\nfor (d in rows()) { }\nfor (String s : it) { }\nfor (i in 0..<n) { }\nfor (k in [a: 1]) { }\nfor (c in 'abc') { }\nfor (x in [f(), g()]) { }\nfor (n = head; n.next != null; n = n.next) { }\nfor (; it.hasNext(); it.next()) { }\nfor (;;) { }\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyForStatement", 4),
                ("EmptyForStatement", 5),
                ("EmptyForStatement", 6),
                ("EmptyForStatement", 10),
            ]
        );
    }

    #[test]
    fn empty_for_counting_on_locals_is_flagged() {
        // an update that only counts is no work; one that reads a field or calls walks
        let src = "for (int i = 0; i < n; i += 1) { }\nfor (i = 0; i < n; i = i + 1) { }\nfor (int i = n; i > 0; i -= 2) { }\nfor (n = head; n.next != null; n = n.next) {}\nfor (; n; n = n[0]) { }\nfor (; i < n; i = step(i)) { }\nfor (; i < n; i++, j++) { }\nfor (; i < n; i++, it.next()) { }\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyForStatement", 1),
                ("EmptyForStatement", 2),
                ("EmptyForStatement", 3),
                ("EmptyForStatement", 7),
            ]
        );
        // a parenthesized literal is a literal; a call in a range's bounds only sizes it
        let src = "for (i in (0..<n)) {}\nfor (x in ([1,2])) {}\nfor (i in 0..<list.size()) {}\nfor (x in (data)) {}\nfor (x in ([f()])) {}\n";
        assert_eq!(
            rules(src),
            vec![
                ("EmptyForStatement", 1),
                ("EmptyForStatement", 2),
                ("EmptyForStatement", 3),
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
        let src = "import a.B\nimport a.B\nimport c.Unused\ndef s = \"x\";\ntry { f() } catch (e) { }\nif (true) { }\nif (a = 1) { f() } else { }\ntry { } finally { }\nwhile (a) { }\nfor (i in 0..2) { }\ndef m = [a: 1, a: 2]\nswitch (x) { case 1: f(); break\n case 1: g() }\nif (x == x) { f() }\nB.go()\n";
        let mut emitted: HashSet<&str> =
            lint(src).expect("parses").iter().map(|v| v.rule).collect();
        let jenkins = "pipeline {\n agent foo\n agent none\n def x = 1\n bogus { }\n options { parameters([]) }\n stages {\n  stage(\"${x}\") {\n   when { onBranch 'x' }\n   steps { if (x) { } }\n  }\n  stage('a') { }\n  stage('a') { steps { echo 1 } }\n }\n}\n";
        let gradle = "repositories { jcenter() }\ndependencies { compile 'a:b:+' }\ntask t << { }\nversion '1'\ndependencies {\n implementation('a:b:1')\n implementation group: 'a', name: 'b'\n}\n";
        let spock = "class S extends Specification {\n def f() {\n  when:\n  f()\n  then: 'x'\n  where:\n  a | b\n  2 | 3 | 4\n }\n def g() {\n  then:\n  x\n }\n def h() {\n  expect:\n  xs.each { it > 0 }\n }\n}\n";
        let more = "package p\nimport p.Q\nimport java.util.Date\nclass K {\n private int pf\n private void m() { }\n static { }\n { }\n boolean equals(Object o) { true }\n int hashcode() { 1 }\n String toString() { null }\n}\nclass L {\n boolean equals(String s) { true }\n}\ndef g() {\n def u = 1\n return 1\n f()\n}\ntry { f() } finally { return 1; throw new E(); assert x }\nswitch (x) { }\ndef s = [1, 1] as Set\ndef t = true ? 1 : 2\nassert true\nif (1 == 2) { f() }\nif (x != null || x.a) { f() }\nif (x % 2 == 1) { f() }\nl.removeAll(l)\nthrow RuntimeException()\nvoid h() { try { f() } catch (e) { new BadException(e) } }\nDate d = new Q()\ndef bd = new BigDecimal(1.5)\nif (a | b) { f() }\ndef nn = !!x\nsynchronized (l) { }\nfor (; c;) { f() }\nInteger.getInteger('p')\ndef r = (int) Math.random()\ndef io = !x instanceof String\ndef m1 = y % 1\nf(a,,b)\n";
        for (name, src) in [
            ("A.groovy", more),
            ("B.groovy", "// groovyfmt-ignore DeadCode\nprintln 1\n"),
            ("C.groovy", "class D {\n}\n"),
            ("Jenkinsfile", jenkins),
            (
                "Jenkinsfile",
                "pipeline {\n agent any\n environment { A = env.B }\n tools { maven }\n options { retry(1); retry(2) }\n stages {\n  stage('a') {\n   input { ok 'x' }\n   when { not { } }\n   agent any\n   matrix { stages { stage('b') { steps { echo 1 } } } }\n  }\n }\n}\n",
            ),
            ("build.gradle", gradle),
            ("build.gradle", "apply plugin: 'maven'\n"),
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
    fn imports_an_include_or_another_import_may_need_are_reported_but_not_fixed() {
        // #192: Hubitat appends the `#include`d library, which uses the file's imports;
        // the last import of a name wins, so removing one can switch the class
        let fixes = |src: &str| -> Vec<(&'static str, usize, bool)> {
            lint(src)
                .expect("parses")
                .into_iter()
                .map(|v| (v.rule, v.line, v.fix.is_some()))
                .collect()
        };
        assert_eq!(
            fixes("import a.B\nimport c.D\nimport c.D\nmetadata { }\n#include x.y\n"),
            vec![
                ("UnusedImport", 1, false),
                ("UnusedImport", 2, false),
                ("DuplicateImport", 3, false)
            ]
        );
        assert_eq!(
            fixes("import p.A\nimport q.A\nimport p.A\nA.v\n"),
            vec![("DuplicateImport", 3, false)]
        );
        assert_eq!(
            fixes("import p.A\nimport p.A\nimport q.B\nA.v\nB.v\n"),
            vec![("DuplicateImport", 2, true)]
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
    fn semicolon_that_keeps_a_method_declaration_away_is_kept() {
        // #211, each checked on Groovy 2.4.21, 3.0.25 and 4.0.24: without the `;`, Groovy 3+
        // reads a top-level type then a call on the next line as a method declaration, an
        // error without a body (`FOO bar()`) and a new method with one (`FOO bar() { 1 }`)
        let semis = |src: &str| {
            lint(src)
                .expect("parses")
                .into_iter()
                .filter(|v| v.rule == "UnnecessarySemicolon")
                .count()
        };
        for src in [
            "FOO;\nbar()\n",
            "Foo;\nbar(int x)\n",
            "FOO;\nbar() { 1 }\n",
            "FOO;\nbar(x) { x }\n",
            "FOO;\nbar()\n{ 1 }\n",
            "a.B;\nbar()\n",
            "java.util.List;\nbar()\n",
            "FOO.BAR;\nbar()\n",
            "Foo.Bar;\nbar(a, b)\n",
            "List<String>;\nbar()\n",
            "Foo<T>;\nbar()\n",
            "String[];\nbar()\n",
            "FOO;\nbar(x)\n",
            "FOO;\nbar(x = 1)\n",
            "FOO;\nbar(Foo)\n",
            "FOO;\nbar(Foo x, int y)\n",
            "FOO;\nbar(it)\n",
            "FOO;\n'bar'()\n",
            "FOO;\n\"bar\"()\n",
            "FOO;\nBar()\n",
            "FOO;\nyield()\n",
            "FOO; // c\n\n// d\nbar()\n",
            "FOO;;\nbar()\n",
            "FOO /* c */;\nbar()\n",
            "FOO;\nbar ()\n",
            "class A {}\nFOO;\nbar()\n",
            // conservative: Groovy reads these two statements, as no argument can be a parameter
            "FOO;\nbar(1)\n",
            "FOO;\nbar(x.y)\n",
        ] {
            assert_eq!(semis(src), 0, "{src:?}");
        }
        // same meaning without the `;` on all three
        for src in [
            "foo;\nbar()\n",
            "a.b;\nbar()\n",
            "A.b;\nbar()\n",
            "this.FOO;\nbar()\n",
            "super.FOO;\nbar()\n",
            "FOO.class;\nbar()\n",
            "L: FOO;\nbar()\n",
            "x = FOO;\nbar()\n",
            "foo();\nbar()\n",
            "FOO();\nbar()\n",
            "String x;\nbar()\n",
            "def x;\nbar()\n",
            "FOO;\nbar x\n",
            "FOO;\nbar\n",
            "FOO;\nbar = 1\n",
            "FOO;\nbar {}\n",
            "FOO;\ndef bar() {}\n",
            // not at the top level, where no method can be declared
            "def m() {\n  FOO;\n  bar()\n}\n",
            "c = {\n  FOO;\n  bar()\n}\n",
            "if (c) {\n  FOO;\n  bar()\n}\n",
        ] {
            assert_eq!(semis(src), 1, "{src:?}");
        }
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
