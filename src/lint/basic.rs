//! CodeNarc's `basic` rules that the syntax tree decides alone, plus the checks from
//! `naming`, `design`, `exceptions` and `unnecessary` that need no type information.
//! Semantics follow each rule's CodeNarc implementation and tests (`org.codenarc.rule.*`);
//! where groovyfmt differs, the doc comment says why.

use super::{Rule, Violation, at, has_interpolation, is_empty_block, is_literal};
use crate::text;
use std::collections::HashSet;
use tree_sitter::Node;

pub(super) const RULES: &[Rule] = &[
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
        name: "NegatedInCheck",
        default: true,
        description: "`!x in xs`, which tests whether the boolean `!x` is in xs",
        why: "`!` binds tighter than `in`, so `!name in ['a', 'b']` asks whether `true` or `false` is in the list: false for this list, whatever `name` is. Groovy 2.4, 3 and 4 all read it so. Put the membership test in parentheses, or use `!in` (Groovy 3 and later).",
        bad: "if (!name in ['admin', 'root']) {\n    allow(name)\n}",
        good: "if (!(name in ['admin', 'root'])) {\n    allow(name)\n}",
        fixable: false,
    },
    Rule {
        name: "UnintendedRegexPattern",
        default: true,
        description: "`s.split('.')`, `s.split('|')`, `s.replaceAll('.', r)` or `s.replaceFirst('|', r)`: a quoted `.` or `|` given where a regular expression is read. Not slashy strings, a `split` on `Pattern.compile(...)` or `Splitter.on(...)`, or `replaceAll('.', c)` with one character `c` that isn't a separator (` `, `=`, `X`), which builds a run as long as the text on purpose; `_ - / : , ; |` still count as a dot meant to become a separator",
        why: "`split`, `replaceAll` and `replaceFirst` read their first argument as a regular expression, where `.` matches any character and `|` matches the empty string. `'a.b'.split('.')` gives an empty array and `'a.b'.replaceAll('.', '-')` gives `---`. Escape it (`'\\\\.'`), or use `tokenize('.')` or `replace('.', '-')`, which take plain text.",
        bad: "def parts = version.split('.')",
        good: "def parts = version.tokenize('.')",
        fixable: false,
    },
    Rule {
        name: "ContradictoryComparison",
        default: true,
        description: "`x == a && x == b` (never true) or `x != a || x != b` (always true) for two different constants a and b (strings, integers, booleans or null) and the same name or property chain x",
        why: "A value can't equal two different constants at once, so `x != 'a' || x != 'b'` is true for every `x` and `x == 1 && x == 2` is never true. The other operator was meant: `x != 'a' && x != 'b'`, or `x == 1 || x == 2`.",
        bad: "if (mode != 'heat' || mode != 'cool') {\n    turnOff()\n}",
        good: "if (mode != 'heat' && mode != 'cool') {\n    turnOff()\n}",
        fixable: false,
    },
    Rule {
        name: "ImpossibleSizeCheck",
        default: true,
        description: "`x.size() >= 0` (always true) or `x.size() < 0` (never true), also with `length()` or with 0 on the left, in the condition of an `if`, `while`, `for`, ternary or elvis. Not an always true one inside `&&` or a never true one inside `||`, where it changes nothing, and not in an assertion or a Spock condition, where tests write it on purpose to check only that the call succeeds",
        why: "A size or a length is never negative, so the comparison has one answer for every `x` and the branch always runs, or never does. Compare with what was meant, often `> 0`.",
        bad: "if (failed.size() >= 0) {\n    report(failed)\n}",
        good: "if (failed.size() > 0) {\n    report(failed)\n}",
        fixable: false,
    },
    Rule {
        name: "CaseConvertedComparison",
        default: true,
        description: "`s.toLowerCase() == 'Text'` with an uppercase ASCII letter in the string, or `s.toUpperCase() == 'text'` with a lowercase one (also `!=` and `equals`)",
        why: "After `toLowerCase()` the value has no uppercase letters, so it never equals a string that has one: `==` is always false and `!=` always true. Write the string in the same case, or use `equalsIgnoreCase`.",
        bad: "if (mode.toLowerCase() == 'Automatic') {\n    scale()\n}",
        good: "if (mode.toLowerCase() == 'automatic') {\n    scale()\n}",
        fixable: false,
    },
    Rule {
        name: "JenkinsEnvBooleanComparison",
        default: true,
        description: "`env.NAME == true` or `env.NAME != false` (either boolean, either side), unless the file declares its own `env` (a variable, parameter, loop variable or `env = ...` binding)",
        why: "Jenkins keeps environment variables as strings, so `env.RELEASE = true` stores `'true'` and `env.RELEASE == true` is always false. Compare with the string (`env.RELEASE == 'true'`), or convert it (`env.RELEASE.toBoolean()`).",
        bad: "if (env.IS_RELEASE == true) {\n    publish()\n}",
        good: "if (env.IS_RELEASE == 'true') {\n    publish()\n}",
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
];

/// Listed after Spock's rules, where it was added: the order of `lint::RULES` shows in
/// `--rules`, SARIF rule indices and docs/lint.md.
pub(super) const LATE_RULES: &[Rule] = &[Rule {
    name: "StringLeftShiftDiscarded",
    default: true,
    description: "`s << x` as a statement on a local String, which builds a new StringBuffer and leaves s unchanged",
    why: "On a String, `<<` returns a new StringBuffer holding both values and never changes the String, so `s << x` on its own line does nothing: whatever it was meant to collect is lost (a test that gathers failures into `s` this way never reports them). Reported only for a local declared `String` or with a plain string literal and never assigned anything else, when the statement's value isn't returned (by a closure, a method or a script). To collect into `s`, start from `new StringBuilder()` or write `s += x`; either changes what later code reading `s` gets, so check it (a string later joined to others may need a separator).",
    bad: "def missing = ''\nfor (name in names) {\n    if (!found(name)) {\n        missing << name << '\\n'\n    }\n}\nif (missing) {\n    throw new IllegalStateException(\"missing:\\n$missing\")\n}",
    good: "def missing = new StringBuilder()\nfor (name in names) {\n    if (!found(name)) {\n        missing << name << '\\n'\n    }\n}\nif (missing) {\n    throw new IllegalStateException(\"missing:\\n$missing\")\n}",
    fixable: false,
}];

/// The node kinds `check` looks at.
pub(super) const KINDS: &[&str] = &[
    "return_statement",
    "throw_statement",
    "break_statement",
    "continue_statement",
    "finally_clause",
    "switch_block",
    "static_initializer",
    "instance_initializer",
    "cast_expression",
    "ternary_expression",
    "elvis_expression",
    "assert_statement",
    "binary_expression",
    "spaceship_expression",
    "method_invocation",
    "class_declaration",
    "method_declaration",
    "constructor_declaration",
    "throw_statement",
    "catch_clause",
    "object_creation_expression",
    "if_statement",
    "while_statement",
    "unary_expression",
    "synchronized_statement",
    "for_statement",
    "instanceof_expression",
    "membership_expression",
    "parenthesized_type_cast",
    "field_declaration",
    "argument_list",
    "list_literal",
    "map_literal",
];

/// Ids of the fields read on every binary expression and call. By name, each lookup
/// searches the grammar's list of field names.
struct Fields {
    operator: u16,
    function: u16,
    field: u16,
}

fn fields(n: Node) -> &'static Fields {
    static FIELDS: std::sync::OnceLock<Fields> = std::sync::OnceLock::new();
    FIELDS.get_or_init(|| {
        let lang = n.language();
        let id = |name: &str| lang.field_id_for_name(name).map_or(0, |f| f.get());
        Fields {
            operator: id("operator"),
            function: id("function"),
            field: id("field"),
        }
    })
}

/// `nodes`: the nodes of the kinds in `KINDS`, in preorder.
pub(super) fn check(src: &str, nodes: &[Node], v: &mut Vec<Violation>) {
    let Some(&first) = nodes.first() else { return };
    let f = fields(first);
    for &n in nodes {
        match n.kind() {
            "return_statement" | "break_statement" | "continue_statement" => dead_code(n, v),
            "throw_statement" => {
                dead_code(n, v);
                missing_new(src, n, v);
            }
            "finally_clause" => finally(n, v),
            "switch_block" if n.named_child_count() == 0 => v.push(at(
                n.parent().unwrap_or(n),
                "EmptySwitchStatement",
                "The switch statement is empty; remove it or add the cases it should handle".into(),
            )),
            "static_initializer" if is_empty_block(n.child_by_field_name("body")) => v.push(at(
                n,
                "EmptyStaticInitializer",
                "The static initializer is empty and can be removed".into(),
            )),
            "instance_initializer" if is_empty_block(n.child_by_field_name("body")) => v.push(at(
                n,
                "EmptyInstanceInitializer",
                "The instance initializer is empty and can be removed".into(),
            )),
            "cast_expression" => {
                duplicate_set_value(src, n, v);
                random_cast(src, n, v);
            }
            "parenthesized_type_cast" => random_cast(src, n, v),
            "ternary_expression" => {
                constant_ternary(src, n, v);
                bitwise_condition(n.child_by_field_name("condition"), v);
            }
            "elvis_expression" => {
                constant_ternary(src, n, v);
                bitwise_condition(n.child_by_field_name("value"), v);
            }
            "if_statement" | "while_statement" => {
                bitwise_condition(n.child_by_field_name("condition"), v);
            }
            "object_creation_expression" => big_decimal(src, n, v),
            "unary_expression" => double_negative(src, n, v),
            "argument_list" | "list_literal" | "map_literal" => empty_element(src, n, v),
            "synchronized_statement" if is_empty_block(n.child_by_field_name("body")) => {
                v.push(at(
                    n,
                    "EmptySynchronizedStatement",
                    "The synchronized statement is empty".into(),
                ));
            }
            "for_statement"
                if n.child_by_field_name("init").is_none()
                    && n.child_by_field_name("update").is_none()
                    && n.child_by_field_name("condition").is_some() =>
            {
                v.push(at(
                    n,
                    "ForLoopShouldBeWhileLoop",
                    "The for loop can be simplified to a while loop".into(),
                ));
            }
            "instanceof_expression" => instanceof_not(src, n, v),
            "membership_expression" => in_not(src, n, v),
            "field_declaration" => random_field(src, n, v),
            "assert_statement" => {
                if n.named_child(0).is_some_and(|c| constant_truth(src, c)) {
                    v.push(at(
                        n,
                        "ConstantAssertExpression",
                        "The assert condition is a constant, so it always passes or always fails; assert what should hold".into(),
                    ));
                }
            }
            "binary_expression" => match n.child_by_field_id(f.operator).map_or("", |o| o.kind()) {
                "%" => mod_one(src, n, v),
                "==" => {
                    two_constants(src, n, v);
                    broken_oddness(src, n, v);
                    case_converted(src, n, v);
                    env_boolean(src, n, v);
                }
                "!=" => {
                    two_constants(src, n, v);
                    case_converted(src, n, v);
                    env_boolean(src, n, v);
                }
                "<" | "<=" | ">" | ">=" => {
                    two_constants(src, n, v);
                    size_check(src, n, v);
                }
                "<<" => string_left_shift(src, n, v),
                "||" | "&&" => {
                    broken_null_check(src, n, v);
                    contradictory(src, n, v);
                }
                _ => {}
            },
            "spaceship_expression" => two_constants(src, n, v),
            "method_invocation" => {
                // `x.removeAll(y)`, `x.equals(y)`, `x.compareTo(y)`
                let method = n
                    .child_by_field_id(f.function)
                    .filter(|x| x.kind() == "field_access")
                    .and_then(|x| x.child_by_field_id(f.field))
                    .map_or("", |x| text(x, src));
                match method {
                    "getInteger" => integer_get_integer(src, n, v),
                    "removeAll" => remove_all_on_self(src, n, v),
                    "equals" => {
                        constant_equals(src, n, v);
                        case_converted(src, n, v);
                    }
                    "compareTo" => constant_equals(src, n, v),
                    "split" | "replaceAll" | "replaceFirst" => regex_pattern(src, n, v),
                    _ => {}
                }
            }
            "class_declaration" => equals_and_hash_code(src, n, v),
            "method_declaration" => {
                method(src, n, v);
                random_returned(src, n, v);
            }
            "constructor_declaration" if is_method(src, n) => method(src, n, v),
            "catch_clause" => exception_not_thrown(src, n, v),
            _ => {}
        }
    }
}

/// StringLeftShiftDiscarded: `s << x` as a statement on a local String. Groovy's
/// `String.leftShift` returns a new StringBuffer and leaves the String alone (checked with
/// Groovy 4.0.24), so the statement does nothing. Only a local declared as `String` or with
/// a plain string literal, never assigned anything but one, and only where the
/// statement's value goes nowhere: not the last statement of a closure, method or script
/// (#212). The message offers fixes without naming one: in geoclimate, `+=` alone would have
/// built broken SQL from a string later joined to others (#229).
fn string_left_shift(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(stmt) = n.parent().filter(|p| p.kind() == "expression_statement") else {
        return;
    };
    let mut left = n;
    while left.kind() == "binary_expression"
        && left
            .child_by_field_name("operator")
            .is_some_and(|o| o.kind() == "<<")
    {
        let Some(l) = left.child_by_field_name("left") else {
            return;
        };
        left = l;
    }
    if left.kind() != "identifier" || value_used(src, stmt) {
        return;
    }
    let name = text(left, src);
    let Some(scope) = string_local(src, stmt, name) else {
        return;
    };
    let mut reassigned = false;
    crate::preorder(scope, |x| {
        reassigned |= x.kind() == "assignment_expression"
            && x.child_by_field_name("left")
                .is_some_and(|l| l.kind() == "identifier" && text(l, src) == name)
            && !(x
                .child_by_field_name("operator")
                .is_some_and(|o| o.kind() == "=")
                && x.child_by_field_name("right")
                    .is_some_and(|r| r.kind() == "string_literal" && !has_interpolation(r)));
        !reassigned
    });
    if !reassigned {
        v.push(at(
            stmt,
            "StringLeftShiftDiscarded",
            format!("`{name}` is a String: `<<` returns a new StringBuffer and leaves `{name}` unchanged, so the statement has no effect. To collect into `{name}`, make it a StringBuilder or write `{name} += ...`, and check what the code reading `{name}` expects"),
        ));
    }
}

/// Whether a statement's value can be used: it is, or ends, what a closure (but one passed to
/// `.each { }` and the like), a non-void method or a script returns, or the statement isn't in
/// a plain block or branch.
fn value_used(src: &str, stmt: Node) -> bool {
    let mut n = stmt;
    while let Some(p) = n.parent() {
        match p.kind() {
            "block" | "closure" | "source_file" => {
                // a script's methods and classes aren't statements: the last statement before
                // them is the script's result (#230)
                let mut next = n.next_named_sibling();
                while next.is_some_and(|x| {
                    is_comment(x)
                        || p.kind() == "source_file"
                            && x.kind().ends_with("_declaration")
                            && !matches!(
                                x.kind(),
                                "local_variable_declaration" | "multi_assignment_declaration"
                            )
                }) {
                    next = next.and_then(|x| x.next_named_sibling());
                }
                if next.is_some() {
                    return false;
                }
                match p.kind() {
                    "closure" => return !super::spock::discarded(p, src),
                    "source_file" => return true,
                    _ => {}
                }
            }
            "if_statement" | "try_statement" | "catch_clause" | "labeled_statement" => {}
            "for_statement" | "for_in_statement" | "while_statement" | "do_while_statement" => {
                return false;
            }
            "method_declaration" => {
                return p
                    .child_by_field_name("return_type")
                    .is_none_or(|t| text(t, src) != "void");
            }
            _ => return true,
        }
        n = p;
    }
    true
}

/// The block declaring `name` as a String local, seen from `at`: the nearest declaration
/// before it, walking out through blocks and closures (whose parameters shadow it).
fn string_local<'a>(src: &str, at: Node<'a>, name: &str) -> Option<Node<'a>> {
    let named = |x: Option<Node>| x.is_some_and(|x| text(x, src) == name);
    let mut n = at;
    while let Some(p) = n.parent() {
        match p.kind() {
            "method_declaration"
            | "constructor_declaration"
            | "lambda_expression"
            | "class_body" => return None,
            "closure" => {
                let mut shadowed = false;
                if let Some(ps) = p
                    .named_child(0)
                    .filter(|c| c.kind() == "closure_parameters")
                {
                    crate::preorder(ps, |x| {
                        shadowed |= x.kind() == "identifier" && text(x, src) == name;
                        !shadowed
                    });
                }
                if shadowed {
                    return None;
                }
            }
            "catch_clause" => {
                if named(
                    p.child_by_field_name("parameter")
                        .and_then(|c| c.child_by_field_name("name")),
                ) {
                    return None;
                }
            }
            "for_in_statement" if named(p.child_by_field_name("variable")) => return None,
            _ => {}
        }
        let mut s = n.prev_named_sibling();
        while let Some(d) = s {
            match d.kind() {
                "local_variable_declaration" => {
                    let ty = d.child_by_field_name("type").map(|t| text(t, src));
                    let mut c = d.walk();
                    let decls: Vec<Node> = if d.child_by_field_name("name").is_some() {
                        vec![d]
                    } else {
                        d.named_children(&mut c)
                            .filter(|x| x.kind() == "variable_declarator")
                            .collect()
                    };
                    if let Some(x) = decls.iter().find(|x| named(x.child_by_field_name("name"))) {
                        let literal = x
                            .child_by_field_name("value")
                            .is_some_and(|v| v.kind() == "string_literal" && !has_interpolation(v));
                        return (ty == Some("String") || ty.is_none() && literal).then_some(p);
                    }
                }
                "multi_assignment_declaration" => {
                    let mut c = d.walk();
                    if d.named_children(&mut c)
                        .any(|x| named(x.child_by_field_name("name")))
                    {
                        return None;
                    }
                }
                _ => {}
            }
            s = d.prev_named_sibling();
        }
        n = p;
    }
    None
}

/// Without wrapping parentheses.
pub(super) fn unparen(mut n: Node) -> Node {
    while n.kind() == "parenthesized_expression"
        && let Some(c) = n.named_child(0)
    {
        n = c;
    }
    n
}

fn is_comment(n: Node) -> bool {
    n.kind().contains("comment")
}

/// DeadCode: a statement after `return`, `throw`, `break` or `continue` in the same block.
/// Declarations are not dead (a script's methods and classes come after its `return`), and
/// neither is a labeled statement, which another path can reach. A `break` right after a
/// `return` or `throw` (the switch-case habit) is redundant but left alone: it is kept on
/// purpose, so that removing the jump can't let the case fall through. A `break` after a
/// `break` or `continue` serves no such purpose and is reported (on real code, a doubled
/// line). Code on the jump's own line is left alone too: the grammar splits some one-line
/// returns in two (`return pipeline { ... }`, `return o instanceof T t && ...`).
///
/// Driven by the jumps, which are few, instead of by every block's statements.
fn dead_code(jump: Node, v: &mut Vec<Violation>) {
    let in_block = jump.parent().is_some_and(|p| {
        matches!(
            p.kind(),
            "block" | "closure" | "switch_case" | "switch_default" | "source_file"
        )
    });
    let Some(s) = next_statement(jump).filter(|_| in_block) else {
        return;
    };
    // after two jumps in a row, the first one reports
    if prev_statement(jump).is_some_and(is_jump) {
        return;
    }
    let k = s.kind();
    let declaration = k.ends_with("_declaration")
        && !matches!(
            k,
            "local_variable_declaration" | "multi_assignment_declaration"
        );
    let habit =
        k == "break_statement" && matches!(jump.kind(), "return_statement" | "throw_statement");
    if !declaration
        && !habit
        && k != "labeled_statement"
        && s.start_position().row > jump.end_position().row
    {
        v.push(at(
            s,
            "DeadCode",
            "This code can never run: it follows a return, throw, break or continue; remove it or move it before".into(),
        ));
    }
}

fn is_jump(n: Node) -> bool {
    matches!(
        n.kind(),
        "return_statement" | "throw_statement" | "break_statement" | "continue_statement"
    )
}

/// The statement after `n` in its block, skipping comments.
fn next_statement(n: Node) -> Option<Node> {
    let mut s = n.next_named_sibling()?;
    while is_comment(s) {
        s = s.next_named_sibling()?;
    }
    Some(s)
}

/// The statement before `n` in its block, skipping comments.
fn prev_statement(n: Node) -> Option<Node> {
    let mut s = n.prev_named_sibling()?;
    while is_comment(s) {
        s = s.prev_named_sibling()?;
    }
    Some(s)
}

/// Whether `n` sits in the code of `top` itself, not in a closure, lambda, method or class
/// declared inside it, whose code runs some other time.
fn same_frame(n: Node, top: Node) -> bool {
    let mut p = n.parent();
    while let Some(a) = p {
        if a == top {
            return true;
        }
        if matches!(
            a.kind(),
            "closure" | "lambda_expression" | "class_body" | "method_declaration"
        ) {
            return false;
        }
        p = a.parent();
    }
    false
}

/// ReturnFromFinallyBlock, ThrowExceptionFromFinallyBlock, AssertWithinFinallyBlock. A
/// `throw` inside a `try` with a `catch`, within the `finally`, may be caught there, so it
/// is left alone.
fn finally(n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else {
        return;
    };
    crate::preorder(body, |x| {
        if x != body && matches!(x.kind(), "closure" | "lambda_expression" | "class_body") {
            return false;
        }
        match x.kind() {
            "return_statement" if same_frame(x, body) => v.push(at(
                x,
                "ReturnFromFinallyBlock",
                "Returning from a finally block discards the exception or return value of the try; move the return out of the finally".into(),
            )),
            "throw_statement" if same_frame(x, body) && !maybe_caught(x, body) => v.push(at(
                x,
                "ThrowExceptionFromFinallyBlock",
                "Throwing from a finally block hides the exception the try may have thrown; handle the error without throwing".into(),
            )),
            "assert_statement" if same_frame(x, body) => v.push(at(
                x,
                "AssertWithinFinallyBlock",
                "A failing assert in a finally block hides the exception the try may have thrown; check before the finally".into(),
            )),
            _ => {}
        }
        true
    });
}

/// `x` is in the body of a `try` with a `catch`, below `top`.
fn maybe_caught(x: Node, top: Node) -> bool {
    let mut p = x.parent();
    let mut child = x;
    while let Some(a) = p {
        if a == top {
            return false;
        }
        if a.kind() == "try_statement"
            && a.child_by_field_name("body") == Some(child)
            && a.children(&mut a.walk())
                .any(|c| c.kind() == "catch_clause")
        {
            return true;
        }
        child = a;
        p = a.parent();
    }
    false
}

/// The value of a constant element, for comparing set elements: `'a'` and `"a"` are equal,
/// `1` and `'1'` are not.
fn element(src: &str, n: Node) -> Option<(bool, String)> {
    if !is_literal(n) {
        return None;
    }
    super::constant_value(n, src).map(|s| (n.kind() == "string_literal", s))
}

/// DuplicateSetValue: `[1, 2, 1] as Set` (any type ending in `Set`).
fn duplicate_set_value(src: &str, n: Node, v: &mut Vec<Violation>) {
    let ty = n.child_by_field_name("type").map_or("", |t| text(t, src));
    let base = ty
        .split('<')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("");
    let Some(list) = n
        .child_by_field_name("value")
        .filter(|l| l.kind() == "list_literal" && base.ends_with("Set"))
    else {
        return;
    };
    let mut seen = HashSet::new();
    let mut c = list.walk();
    for e in list.named_children(&mut c) {
        if let Some(k) = element(src, e)
            && !seen.insert(k)
        {
            v.push(at(
                e,
                "DuplicateSetValue",
                format!(
                    "The value {} appears more than once in this {base}, which keeps only one; remove the copy or fix the value",
                    text(e, src)
                ),
            ));
        }
    }
}

/// A condition whose truth is fixed: a literal, `Boolean.TRUE` / `Boolean.FALSE`, or a list
/// or map literal (true unless empty) without a spread.
fn constant_truth(src: &str, n: Node) -> bool {
    let n = unparen(n);
    match n.kind() {
        "list_literal" | "map_literal" => {
            let mut c = n.walk();
            n.named_children(&mut c)
                .all(|e| !matches!(e.kind(), "spread_arguments" | "spread_map_entry"))
        }
        "field_access" => matches!(text(n, src), "Boolean.TRUE" | "Boolean.FALSE"),
        _ => is_literal(n),
    }
}

/// ConstantTernaryExpression: `true ? a : b`, `'x' ?: y`.
fn constant_ternary(src: &str, n: Node, v: &mut Vec<Violation>) {
    let field = if n.kind() == "ternary_expression" {
        "condition"
    } else {
        "value"
    };
    if n.child_by_field_name(field)
        .is_some_and(|c| constant_truth(src, c))
    {
        v.push(at(
            n,
            "ConstantTernaryExpression",
            "The condition of this ternary is a constant, so it always picks the same branch; use that branch directly".into(),
        ));
    }
}

/// A value made only of constants: literals, `Boolean.TRUE`, and list and map literals of them.
fn constant_value(src: &str, n: Node) -> bool {
    let mut ok = true;
    crate::preorder(unparen(n), |x| {
        ok &= match x.kind() {
            "list_literal" | "map_literal" | "map_entry" | "parenthesized_expression" => true,
            "number_literal" | "boolean_literal" | "null_literal" => true,
            "string_literal" => !has_interpolation(x),
            // a map key `a:` is a string
            "identifier" => x.parent().is_some_and(|p| {
                p.kind() == "map_entry" && p.child_by_field_name("key") == Some(x)
            }),
            "field_access" => matches!(text(x, src), "Boolean.TRUE" | "Boolean.FALSE"),
            _ => !x.is_named() || x.kind() == "string_fragment",
        };
        ok && x.kind() != "field_access" && x.kind() != "string_literal"
    });
    ok
}

/// ComparisonOfTwoConstants: `1 == 2`, `[a: 1] == [a: 2]`.
fn two_constants(src: &str, n: Node, v: &mut Vec<Violation>) {
    let op = n.child_by_field_name("operator").map_or("", |o| o.kind());
    if n.kind() == "binary_expression" && !matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=") {
        return;
    }
    if let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) && constant_value(src, l)
        && constant_value(src, r)
    {
        v.push(at(
            n,
            "ComparisonOfTwoConstants",
            format!(
                "Comparing two constants always gives the same answer: {}; compare the value that varies",
                text(n, src)
            ),
        ));
    }
}

/// ComparisonOfTwoConstants in its method forms: `[1, 2].equals([3])`, `'a'.compareTo('b')`.
fn constant_equals(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(f) = n
        .child_by_field_name("function")
        .filter(|f| f.kind() == "field_access")
    else {
        return;
    };
    if !f
        .child_by_field_name("field")
        .is_some_and(|x| matches!(text(x, src), "equals" | "compareTo"))
    {
        return;
    }
    let args = n
        .child_by_field_name("arguments")
        .filter(|a| a.kind() == "argument_list");
    if let (Some(o), Some(a)) = (f.child_by_field_name("object"), args)
        && a.named_child_count() == 1
        && constant_value(src, o)
        && a.named_child(0).is_some_and(|x| constant_value(src, x))
    {
        v.push(at(
            n,
            "ComparisonOfTwoConstants",
            format!(
                "Comparing two constants always gives the same answer: {}; compare the value that varies",
                text(n, src)
            ),
        ));
    }
}

/// Whether a `constructor_declaration` is really a method: the grammar reads a method with
/// modifiers and no return type (`private static helper() { }`) as a constructor unless its
/// name is the class's.
pub(super) fn is_method(src: &str, n: Node) -> bool {
    let class = n
        .parent()
        .and_then(|b| b.parent())
        .and_then(|c| c.child_by_field_name("name"));
    n.child_by_field_name("name")
        .is_some_and(|m| class.is_none_or(|c| text(c, src) != text(m, src)))
}

/// The name `x` in `x == null` / `null != x`, for `==` or `!=` as `op`.
fn null_tested<'a>(src: &'a str, n: Node, op: &str) -> Option<&'a str> {
    let n = unparen(n);
    if n.kind() != "binary_expression"
        || n.child_by_field_name("operator").map(|o| o.kind()) != Some(op)
    {
        return None;
    }
    let (l, r) = (
        n.child_by_field_name("left")?,
        n.child_by_field_name("right")?,
    );
    let x = match (l.kind(), r.kind()) {
        (_, "null_literal") => l,
        ("null_literal", _) => r,
        _ => return None,
    };
    is_path(x).then(|| text(x, src))
}

/// A variable or a chain of plain property reads, `a` or `this.a.b`: the same text means the
/// same value.
fn is_path(n: Node) -> bool {
    match n.kind() {
        "identifier" => true,
        "field_access" => {
            n.child_by_field_name("object").is_some_and(is_path)
                && n.child_by_field_name("field")
                    .is_some_and(|f| f.kind() == "identifier")
        }
        _ => false,
    }
}

/// The object a dereference reads, after any `!` and down the left of comparisons and
/// boolean operators: `x` in `x.size() > 0`, `!x.empty`, `x.a && y`.
fn dereferenced(n: Node) -> Option<Node> {
    let mut n = unparen(n);
    loop {
        n = match n.kind() {
            "unary_expression" => n.child_by_field_name("operand")?,
            "binary_expression" => n.child_by_field_name("left")?,
            "parenthesized_expression" => n.named_child(0)?,
            "method_invocation" => {
                let f = n.child_by_field_name("function")?;
                if f.kind() != "field_access" {
                    return None;
                }
                return f.child_by_field_name("object");
            }
            "field_access" | "subscript_expression" => return n.child_by_field_name("object"),
            _ => return None,
        };
    }
}

/// BrokenNullCheck: `x != null || x.a`, `x == null && x.a`, where the right side
/// dereferences exactly when `x` is null.
fn broken_null_check(src: &str, n: Node, v: &mut Vec<Violation>) {
    let op = n.child_by_field_name("operator").map_or("", |o| o.kind());
    let test = match op {
        "||" => "!=",
        "&&" => "==",
        _ => return,
    };
    let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) else {
        return;
    };
    if let Some(x) = null_tested(src, l, test)
        && dereferenced(r).is_some_and(|o| text(o, src) == x)
    {
        v.push(at(
            n,
            "BrokenNullCheck",
            format!(
                "`{x}` is dereferenced exactly when it is null, which throws a NullPointerException; did you mean {}?",
                if op == "||" { "&&" } else { "||" }
            ),
        ));
    }
}

/// BrokenOddnessCheck: `x % 2 == 1`, false for negative odd numbers.
fn broken_oddness(src: &str, n: Node, v: &mut Vec<Violation>) {
    if n.child_by_field_name("operator").map(|o| o.kind()) != Some("==") {
        return;
    }
    let is_mod2 = |m: Node| {
        let m = unparen(m);
        m.kind() == "binary_expression"
            && m.child_by_field_name("operator").map(|o| o.kind()) == Some("%")
            && m.child_by_field_name("right")
                .is_some_and(|r| text(r, src) == "2")
    };
    let is_one = |m: Node| text(unparen(m), src) == "1";
    if let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) && ((is_mod2(l) && is_one(r)) || (is_one(l) && is_mod2(r)))
    {
        v.push(at(
            n,
            "BrokenOddnessCheck",
            format!(
                "`{}` is false for negative odd numbers (-3 % 2 is -1); use `% 2 != 0`",
                text(n, src)
            ),
        ));
    }
}

/// RemoveAllOnSelf: `list.removeAll(list)`.
fn remove_all_on_self(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(f) = n
        .child_by_field_name("function")
        .filter(|f| f.kind() == "field_access")
    else {
        return;
    };
    if f.child_by_field_name("field").map(|x| text(x, src)) != Some("removeAll") {
        return;
    }
    let Some(o) = f.child_by_field_name("object").filter(|o| is_path(*o)) else {
        return;
    };
    let args = n.child_by_field_name("arguments");
    if let Some(a) = args
        && a.named_child_count() == 1
        && a.named_child(0)
            .is_some_and(|x| text(x, src) == text(o, src))
    {
        v.push(at(
            n,
            "RemoveAllOnSelf",
            format!(
                "`{}` empties the collection less clearly than `{}.clear()`, and can throw ConcurrentModificationException; use clear()",
                text(n, src),
                text(o, src)
            ),
        ));
    }
}

/// Whether `n` (a declaration) has the modifier token `m`.
pub(super) fn has_modifier(n: Node, m: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c).any(|k| k.kind() == m)
}

/// The simple names of the annotations on a declaration.
pub(super) fn annotations<'a>(src: &'a str, n: Node) -> Vec<&'a str> {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|a| a.kind() == "annotation")
        .filter_map(|a| a.child_by_field_name("name"))
        .map(|a| text(a, src).rsplit('.').next().unwrap_or(""))
        .collect()
}

/// A method's parameters.
fn params(m: Node) -> Vec<Node> {
    m.child_by_field_name("parameters")
        .map_or_else(Vec::new, |p| {
            let mut c = p.walk();
            p.named_children(&mut c)
                .filter(|x| x.kind() == "formal_parameter")
                .collect()
        })
}

/// A parameter declared `Object`, `java.lang.Object`, `def` or untyped.
fn object_param(src: &str, p: Node) -> bool {
    p.child_by_field_name("type")
        .is_none_or(|t| matches!(text(t, src), "Object" | "java.lang.Object" | "def"))
}

fn method_name<'a>(src: &'a str, m: Node) -> &'a str {
    m.child_by_field_name("name").map_or("", |n| text(n, src))
}

/// EqualsAndHashCode: a class that defines `equals(Object)` without `hashCode()`. Not when an
/// annotation generates it (`@EqualsAndHashCode`, `@Canonical`, `@Immutable`, `@Value`-style
/// transforms). CodeNarc also flags `hashCode()` without `equals`, but that keeps the
/// contract (identity equality, any hash), and on real code it only found deliberate test
/// fixtures, so groovyfmt doesn't.
fn equals_and_hash_code(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else {
        return;
    };
    let (mut equals, mut hash) = (None, false);
    let mut c = body.walk();
    for m in body.named_children(&mut c).filter(|m| {
        matches!(m.kind(), "method_declaration" | "constructor_declaration")
            && matches!(method_name(src, *m), "equals" | "hashCode")
            && !has_modifier(*m, "static")
    }) {
        let p = params(m);
        match method_name(src, m) {
            "equals" if p.len() == 1 && object_param(src, p[0]) => equals = Some(m),
            "hashCode" if p.is_empty() => hash = true,
            _ => {}
        }
    }
    let Some(equals) = equals.filter(|_| !hash) else {
        return;
    };
    if identity_only(src, equals)
        || (has_modifier(n, "abstract") && calls_abstract(src, body, equals))
    {
        return;
    }
    let generated = annotations(src, n).iter().any(|a| {
        matches!(
            *a,
            "EqualsAndHashCode" | "Canonical" | "Immutable" | "Value" | "AutoValue" | "Data"
        )
    });
    // a superclass can supply the hashCode()
    let mut c = n.walk();
    if !generated && !n.children(&mut c).any(|k| k.kind() == "superclass") {
        let name = n.child_by_field_name("name").map_or("", |x| text(x, src));
        v.push(at(
            n.child_by_field_name("name").unwrap_or(n),
            "EqualsAndHashCode",
            format!(
                "Class {name} defines equals(Object) but not hashCode(); equal objects must have equal hash codes, so define hashCode() too"
            ),
        ));
    }
}

/// Whether `equals(Object)` is never broader than identity, so the inherited identity
/// `hashCode()` keeps the contract: its body is one statement, optionally `return`, of
/// `false`, `this.is(o)`, `o.is(this)`, `DefaultGroovyMethods.is(this, o)` and `o == null`,
/// joined by `||` (grails-core's `VersionComparator` and `NullSafeNavigator`).
fn identity_only(src: &str, equals: Node) -> bool {
    let (Some(p), Some(body)) = (
        params(equals)
            .first()
            .and_then(|p| p.child_by_field_name("name")),
        equals.child_by_field_name("body"),
    ) else {
        return false;
    };
    let p = text(p, src);
    let mut c = body.walk();
    let stmts: Vec<Node> = body
        .named_children(&mut c)
        .filter(|s| !is_comment(*s))
        .collect();
    let [s] = stmts[..] else { return false };
    if !matches!(s.kind(), "return_statement" | "expression_statement") {
        return false;
    }
    fn identity(src: &str, p: &str, e: Node) -> bool {
        let e = unparen(e);
        let side = |f| e.child_by_field_name(f);
        match e.kind() {
            "boolean_literal" => text(e, src) == "false",
            "binary_expression" => match e.child_by_field_name("operator").map_or("", |o| o.kind())
            {
                "||" => [side("left"), side("right")]
                    .iter()
                    .all(|x| x.is_some_and(|x| identity(src, p, x))),
                "==" => {
                    let t = [side("left"), side("right")].map(|x| x.map_or("", |x| text(x, src)));
                    t == ["null", p] || t == [p, "null"]
                }
                _ => false,
            },
            "method_invocation" => {
                let Some(f) = side("function") else {
                    return false;
                };
                let mut ops: Vec<&str> = side("arguments")
                    .map(|a| {
                        a.named_children(&mut a.walk())
                            .map(|x| text(x, src))
                            .collect()
                    })
                    .unwrap_or_default();
                let name = if f.kind() == "field_access" {
                    let object = f.child_by_field_name("object").map_or("", |o| text(o, src));
                    if object != "DefaultGroovyMethods" {
                        ops.push(object);
                    }
                    f.child_by_field_name("field").map_or("", |x| text(x, src))
                } else {
                    text(f, src)
                };
                name == "is" && (ops == [p, "this"] || ops == ["this", p])
            }
            _ => false,
        }
    }
    s.named_child(0).is_some_and(|e| identity(src, p, e))
}

/// Whether `equals` calls one of the methods its class declares abstract: equality is left
/// to the subclasses (hoist-core's `Filter`), so `hashCode()` belongs with them too.
fn calls_abstract(src: &str, class_body: Node, equals: Node) -> bool {
    let mut c = class_body.walk();
    let abstract_methods: HashSet<&str> = class_body
        .named_children(&mut c)
        .filter(|m| m.kind() == "method_declaration" && has_modifier(*m, "abstract"))
        .map(|m| method_name(src, m))
        .collect();
    let mut calls = false;
    if let Some(body) = equals.child_by_field_name("body") {
        crate::preorder(body, |x| {
            if x.kind() == "method_invocation"
                && let Some(f) = x.child_by_field_name("function")
            {
                let name = match f.kind() {
                    "identifier" => text(f, src),
                    "field_access"
                        if f.child_by_field_name("object")
                            .is_some_and(|o| text(o, src) == "this") =>
                    {
                        f.child_by_field_name("field").map_or("", |x| text(x, src))
                    }
                    _ => "",
                };
                calls |= abstract_methods.contains(name);
            }
            !calls
        });
    }
    calls
}

/// Whether class `c`, or a superclass declared in the same file, declares `equals(Object)`,
/// which can delegate to a typed overload.
fn declares_object_equals(src: &str, mut c: Node) -> bool {
    let mut root = c;
    while let Some(p) = root.parent() {
        root = p;
    }
    // ponytail: a bounded walk up `extends`, which also stops a cycle
    for _ in 0..8 {
        let Some(body) = c.child_by_field_name("body") else {
            return false;
        };
        let mut w = body.walk();
        if body.named_children(&mut w).any(|m| {
            matches!(m.kind(), "method_declaration" | "constructor_declaration")
                && method_name(src, m) == "equals"
                && !has_modifier(m, "static")
                && matches!(params(m)[..], [p] if object_param(src, p))
        }) {
            return true;
        }
        let Some(sup) = c
            .children(&mut c.walk())
            .find(|k| k.kind() == "superclass")
            .and_then(|s| {
                s.children(&mut s.walk())
                    .filter(|t| t.is_named() && t.kind() != "annotation")
                    .last()
            })
            .map(|t| {
                let t = text(t, src).split('<').next().unwrap_or("");
                t.rsplit('.').next().unwrap_or("").trim()
            })
        else {
            return false;
        };
        let mut next = None;
        crate::preorder(root, |x| {
            if next.is_none()
                && x.kind() == "class_declaration"
                && x.child_by_field_name("name")
                    .is_some_and(|n| text(n, src) == sup)
            {
                next = Some(x);
            }
            next.is_none()
        });
        let Some(next) = next else { return false };
        c = next;
    }
    false
}

/// EqualsOverloaded, ObjectOverrideMisspelledMethodName, ToStringReturnsNull.
fn method(src: &str, m: Node, v: &mut Vec<Violation>) {
    let name = method_name(src, m);
    // only equal(s), hashCode and toString in any case
    if !matches!(name.len(), 5 | 6 | 8) || has_modifier(m, "static") {
        return;
    }
    let p = params(m);
    let at_name = |rule, msg: String| at(m.child_by_field_name("name").unwrap_or(m), rule, msg);
    // only the one-parameter form looks like an override; `equals(a, b)` implementing an
    // interface (Hibernate's UserType) or a Spock feature named `equals` is something else
    // nor an abstract declaration, nor an overload that a class's equals(Object) can call
    if name == "equals"
        && p.len() == 1
        && !object_param(src, p[0])
        && m.child_by_field_name("body").is_some()
        // implements a base class's own typed hook (jjwt's `equals(Jwk)`, #212)
        && !annotations(src, m).contains(&"Override")
        && !m
            .parent()
            .and_then(|b| b.parent())
            .is_some_and(|c| declares_object_equals(src, c))
    {
        v.push(at_name(
            "EqualsOverloaded",
            "This equals() takes a typed parameter, so it overloads equals instead of overriding it, and == and collections ignore it; declare equals(Object)".into(),
        ));
    }
    let lower = name.to_ascii_lowercase();
    let misspelled = match lower.as_str() {
        "equals" | "equal" => name != "equals" && p.len() == 1 && object_param(src, p[0]),
        "hashcode" => name != "hashCode" && p.is_empty(),
        "tostring" => name != "toString" && p.is_empty(),
        _ => false,
    };
    if misspelled {
        let right = match lower.as_str() {
            "hashcode" => "hashCode",
            "tostring" => "toString",
            _ => "equals",
        };
        v.push(at_name(
            "ObjectOverrideMisspelledMethodName",
            format!("`{name}` looks like a misspelling of {right}, so it doesn't override Object.{right}; rename it to {right}"),
        ));
    }
    if name == "toString"
        && p.is_empty()
        && let Some(body) = m.child_by_field_name("body")
    {
        to_string_returns_null(body, v);
    }
}

/// ToStringReturnsNull: `return null` in toString(), a last statement `null`, or an empty
/// body (which returns null).
fn to_string_returns_null(body: Node, v: &mut Vec<Violation>) {
    let msg = || {
        "toString() returns null here, which breaks string concatenation and logging; return a string".to_string()
    };
    let mut c = body.walk();
    let stmts: Vec<Node> = body
        .named_children(&mut c)
        .filter(|s| !is_comment(*s))
        .collect();
    match stmts.last() {
        None => v.push(at(body, "ToStringReturnsNull", msg())),
        Some(s)
            if s.kind() == "expression_statement"
                && s.named_child(0)
                    .is_some_and(|e| unparen(e).kind() == "null_literal") =>
        {
            v.push(at(*s, "ToStringReturnsNull", msg()))
        }
        _ => {}
    }
    crate::preorder(body, |x| {
        if matches!(x.kind(), "closure" | "lambda_expression" | "class_body") {
            return false;
        }
        if x.kind() == "return_statement"
            && x.named_child(0)
                .is_some_and(|e| unparen(e).kind() == "null_literal")
        {
            v.push(at(x, "ToStringReturnsNull", msg()));
        }
        true
    });
}

/// A simple class name that reads as an exception: capitalized, ending in `Exception`,
/// `Failure` or `Fault`.
fn exception_like(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && ["Exception", "Failure", "Fault"]
            .iter()
            .any(|s| name.ends_with(s))
}

/// MissingNewInThrowStatement: `throw RuntimeException('x')`, which calls a method that
/// doesn't exist instead of creating the exception.
fn missing_new(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(e) = n.named_child(0).map(unparen) else {
        return;
    };
    // `throw RuntimeException('x')`, or `throw RuntimeException`, which throws a Class
    let f = if e.kind() == "method_invocation" {
        e.child_by_field_name("function")
    } else {
        Some(e)
    };
    if let Some(f) = f
        && f.kind() == "identifier"
        && exception_like(text(f, src))
    {
        v.push(at(
            e,
            "MissingNewInThrowStatement",
            format!(
                "This throw is missing `new`: it calls a method named {} (or throws the class itself) instead of creating the exception; write `throw new {0}(...)`",
                text(f, src)
            ),
        ));
    }
}

/// ExceptionNotThrown: a catch block ending in `new SomeException(...)` that isn't thrown.
/// Not when that value is what the method or closure returns (Groovy returns the last
/// expression), unless the method is `void`.
fn exception_not_thrown(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(body) = n.child_by_field_name("body") else {
        return;
    };
    let mut c = body.walk();
    let Some(last) = body
        .named_children(&mut c)
        .filter(|s| !is_comment(*s))
        .last()
    else {
        return;
    };
    let Some(e) = last.named_child(0).filter(|e| {
        last.kind() == "expression_statement" && e.kind() == "object_creation_expression"
    }) else {
        return;
    };
    let ty = e.child_by_field_name("type").map_or("", |t| text(t, src));
    let simple = ty
        .split('<')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("");
    if !simple.ends_with("Exception") || e.child_by_field_name("body").is_some() || returned(src, n)
    {
        return;
    }
    v.push(at(
        e,
        "ExceptionNotThrown",
        format!("This creates a {simple} and drops it; did you mean `throw new {simple}(...)`?"),
    ));
}

/// Whether the value of the catch clause `n` can be the value its method or closure returns:
/// the `try` is the last statement there, possibly through more blocks and branches.
fn returned(src: &str, n: Node) -> bool {
    let mut x = n.parent(); // the try
    while let Some(s) = x {
        let Some(p) = s.parent() else { return false };
        match p.kind() {
            "block" | "closure" => {
                let mut c = p.walk();
                let last = p.named_children(&mut c).filter(|k| !is_comment(*k)).last();
                if last != Some(s) {
                    return false;
                }
                let Some(owner) = p.parent().filter(|_| p.kind() == "block") else {
                    // a closure returns its last value
                    return true;
                };
                match owner.kind() {
                    "method_declaration" => {
                        return owner
                            .child_by_field_name("return_type")
                            .is_none_or(|t| text(t, src) != "void");
                    }
                    "constructor_declaration" | "static_initializer" | "instance_initializer" => {
                        return false;
                    }
                    "lambda_expression" => return true,
                    _ => x = Some(owner),
                }
            }
            "if_statement" | "labeled_statement" | "try_statement" | "catch_clause"
            | "finally_clause" | "switch_case" | "switch_default" | "switch_block" => x = Some(p),
            "source_file" => return true,
            _ => return false,
        }
    }
    false
}

/// BigDecimalInstantiation: `new BigDecimal(1.1)` or `new java.math.BigDecimal(1.1d)`, a
/// BigDecimal made from a double, which is inexact. As in CodeNarc, the first argument must
/// be a decimal literal (a BigDecimal or double in Groovy); `1.1f` and integers are fine.
fn big_decimal(src: &str, n: Node, v: &mut Vec<Violation>) {
    let ty = n.child_by_field_name("type").map_or("", |t| text(t, src));
    if !matches!(ty, "BigDecimal" | "java.math.BigDecimal")
        || n.child_by_field_name("body").is_some()
    {
        return;
    }
    let Some(arg) = n
        .child_by_field_name("arguments")
        .and_then(|a| a.named_children(&mut a.walk()).find(|c| !is_comment(*c)))
    else {
        return;
    };
    let t = text(arg, src);
    let lower = t.to_ascii_lowercase();
    let decimal = arg.kind() == "number_literal"
        && !lower.starts_with("0x")
        && !lower.starts_with("0b")
        && (lower.contains('.') || lower.contains('e') || lower.ends_with('d'))
        && !lower.ends_with('f');
    if decimal {
        let number = t.trim_end_matches(['d', 'D', 'g', 'G']);
        v.push(at(
            n,
            "BigDecimalInstantiation",
            format!(
                "Call to {} uses the double constructor and should probably be replaced with new {ty}(\"{number}\")",
                text(n, src)
            ),
        ));
    }
}

/// BitwiseOperatorInConditional: `|` or `&` in the condition of an `if`, `while`, ternary
/// or elvis, found as CodeNarc does: through the binary operators the condition is made of,
/// not into calls or `!` (parentheses don't count, Groovy's syntax tree drops them).
fn bitwise_condition(c: Option<Node>, v: &mut Vec<Violation>) {
    let mut stack: Vec<Node> = c.into_iter().collect();
    while let Some(n) = stack.pop() {
        let n = unparen(n);
        if n.kind() != "binary_expression" {
            continue;
        }
        match n.child_by_field_name("operator").map_or("", |o| o.kind()) {
            "|" => v.push(at(
                n,
                "BitwiseOperatorInConditional",
                "Use of a bitwise or (|) operator in a conditional".into(),
            )),
            "&" => v.push(at(
                n,
                "BitwiseOperatorInConditional",
                "Use of a bitwise and (&) operator in a conditional".into(),
            )),
            _ => stack.extend(
                ["right", "left"]
                    .iter()
                    .filter_map(|f| n.child_by_field_name(f)),
            ),
        }
    }
}

fn is_not(n: Node) -> bool {
    n.kind() == "unary_expression"
        && n.child_by_field_name("operator")
            .is_some_and(|o| o.kind() == "!")
}

/// DoubleNegative: `!!x` or `!(!x)`.
fn double_negative(src: &str, n: Node, v: &mut Vec<Violation>) {
    if is_not(n)
        && n.child_by_field_name("operand")
            .is_some_and(|o| is_not(unparen(o)))
    {
        v.push(at(
            n,
            "DoubleNegative",
            format!(
                "The expression {} is a confusing double negative",
                text(n, src)
            ),
        ));
    }
}

/// Groovy24EmptyElement: `foo(a,,b)`, `[a, , b]`, `[a: 1,, b: 2]`, reported at each comma
/// that follows another one. Groovy 2.4 drops one empty element, but rejects a second one,
/// or one with a trailing comma (`[1,,2,]`), as Groovy 3 and later reject them all. So when
/// it is the only one, without a trailing comma, the fix removes that comma, with the
/// whitespace before it back to the other comma (only the comma when a comment is between):
/// the call, list or map Groovy 2.4 compiled. Code that compiled nowhere isn't fixed.
fn empty_element(src: &str, n: Node, v: &mut Vec<Violation>) {
    let what = match n.kind() {
        "argument_list" => "argument",
        "list_literal" => "list element",
        _ => "map entry",
    };
    let kids: Vec<Node> = n
        .children(&mut n.walk())
        .filter(|k| !k.kind().contains("comment"))
        .collect();
    let extra: Vec<(Node, Node)> = kids
        .windows(2)
        .filter(|w| w[0].kind() == "," && w[1].kind() == ",")
        .map(|w| (w[0], w[1]))
        .collect();
    let trailing = kids.len() >= 2 && kids[kids.len() - 2].kind() == ",";
    let fixable = extra.len() == 1 && !trailing;
    for &(c, k) in &extra {
        let start = if src[c.end_byte()..k.start_byte()].trim().is_empty() {
            c.end_byte()
        } else {
            k.start_byte()
        };
        v.push(Violation {
            fix: fixable.then(|| super::Edit {
                range: start..k.end_byte(),
                with: String::new(),
            }),
            ..at(
                k,
                "Groovy24EmptyElement",
                format!("An empty {what}: Groovy 2.4 drops it and Groovy 3 and later reject it; remove the extra comma"),
            )
        });
    }
}

/// UnnecessaryInstanceOfCheck: `!x instanceof T`, which tests the boolean `!x`.
fn instanceof_not(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(x) = n
        .child_by_field_name("value")
        .map(unparen)
        .filter(|x| is_not(*x))
    else {
        return;
    };
    let ty = n.child_by_field_name("type").map_or("", |t| text(t, src));
    let what = if ty == "Boolean" {
        "will always be a Boolean".to_string()
    } else {
        format!("will never be a {ty}")
    };
    v.push(at(
        n,
        "UnnecessaryInstanceOfCheck",
        format!("The result of '{}' {what}", text(x, src)),
    ));
}

/// NegatedInCheck: `!x in xs`, which asks whether the boolean `!x` is in `xs` (Groovy 2.4, 3
/// and 4 all read it so, #230).
fn in_not(src: &str, n: Node, v: &mut Vec<Violation>) {
    let Some(e) = n.child_by_field_name("element").filter(|x| is_not(*x)) else {
        return;
    };
    let Some(x) = e.child_by_field_name("operand") else {
        return;
    };
    // the `!in` token takes the character after it, so it is read from the text
    let op = if src[e.end_byte()..].trim_start().starts_with("!in") {
        "!in"
    } else {
        "in"
    };
    let x = text(x, src);
    v.push(at(
        n,
        "NegatedInCheck",
        format!("`!` applies to `{x}` alone, so this asks whether the boolean `!{x}` is a member; write `!({x} {op} ...)`"),
    ));
}

/// The text of a quoted string literal (`'...'`, `"..."` or a triple-quoted one) with
/// nothing interpolated and no escapes. `None` for a slashy string, whose author chose a
/// regular expression, and for anything else.
fn plain_quoted(src: &str, n: Node) -> Option<String> {
    let n = unparen(n);
    if n.kind() != "string_literal" || has_interpolation(n) {
        return None;
    }
    let t = text(n, src);
    let q = ["'''", "\"\"\"", "'", "\""]
        .into_iter()
        .find(|q| t.len() >= 2 * q.len() && t.starts_with(q) && t.ends_with(q))?;
    let body = &t[q.len()..t.len() - q.len()];
    (!body.contains('\\')).then(|| body.to_string())
}

/// The method a call names: `name` in `x.name(...)`.
fn called_method<'a>(src: &'a str, n: Node) -> Option<&'a str> {
    n.child_by_field_name("function")
        .filter(|f| f.kind() == "field_access")
        .and_then(|f| f.child_by_field_name("field"))
        .map(|f| text(f, src))
}

/// The arguments of a call in parentheses; `None` for a closure or command argument.
fn call_args(n: Node) -> Option<Node> {
    n.child_by_field_name("arguments")
        .filter(|a| a.kind() == "argument_list")
}

/// The node above `n`, past any parentheses.
fn parent_past_parens(n: Node) -> Option<Node> {
    let mut up = n.parent();
    while let Some(p) = up.filter(|p| p.kind() == "parenthesized_expression") {
        up = p.parent();
    }
    up
}

fn operator(n: Node) -> &'static str {
    n.child_by_field_name("operator").map_or("", |o| o.kind())
}

/// UnintendedRegexPattern: `s.split('.')`, `s.replaceAll('|', r)`: a quoted `.` or `|`
/// given where Java reads a regular expression (SpotBugs' RE_POSSIBLE_UNINTENDED_PATTERN).
/// `replaceAll('.', c)` with one character `c` that isn't a separator builds a run as long as
/// the text, on purpose (an underline of `=` under a header, in GradleUtils, or a mask of `X`);
/// one of `_ - / : , ; |` is a dot meant to become a separator (bigtop, kordamp). A `split` on
/// `Pattern.compile(...)` or Guava's `Splitter.on(...)` takes the text, not a pattern.
fn regex_pattern(src: &str, n: Node, v: &mut Vec<Violation>) {
    let (Some(method), Some(args)) = (called_method(src, n), call_args(n)) else {
        return;
    };
    let receiver = n
        .child_by_field_name("function")
        .and_then(|f| f.child_by_field_name("object"))
        .map(unparen);
    if receiver.is_some_and(|r| {
        r.kind() == "method_invocation"
            && matches!(
                called_method(src, r),
                Some("compile" | "on" | "onPattern" | "fixedLength")
            )
    }) {
        return;
    }
    let count = args.named_child_count();
    if count != 2 && !(method == "split" && count == 1) {
        return;
    }
    let Some(p) = args.named_child(0).and_then(|a| plain_quoted(src, a)) else {
        return;
    };
    let what = match p.as_str() {
        "." => "`.` matches any character",
        "|" => "`|` matches the empty string",
        _ => return,
    };
    if method != "split" {
        match args.named_child(1).and_then(|r| plain_quoted(src, r)) {
            Some(r)
                if !(p == "."
                    && r.chars().count() == 1
                    && !matches!(r.as_str(), "_" | "-" | "/" | ":" | "," | ";" | "|" | ".")) => {}
            _ => return,
        }
    }
    let plain = if method == "split" {
        "tokenize"
    } else {
        "replace"
    };
    v.push(at(
        n,
        "UnintendedRegexPattern",
        format!("{method} reads '{p}' as a regular expression, where {what}; escape it ('\\\\{p}') or use {plain}, which takes plain text"),
    ));
}

/// A constant for ContradictoryComparison: a plain quoted string, an integer, a boolean or
/// null, tagged by kind. Decimals are left out: Groovy compares `1 == 1.0` by value.
fn comparison_constant(src: &str, n: Node) -> Option<String> {
    let n = unparen(n);
    match n.kind() {
        "null_literal" => Some("null".into()),
        "boolean_literal" => Some(format!("b{}", text(n, src))),
        "string_literal" => plain_quoted(src, n).map(|s| format!("s{s}")),
        "number_literal" => {
            let t = text(n, src).replace('_', "");
            let t = t.trim_end_matches(['l', 'L', 'i', 'I', 'g', 'G']);
            let (digits, radix) = match t.get(..2) {
                Some("0x" | "0X") => (&t[2..], 16),
                Some("0b" | "0B") => (&t[2..], 2),
                _ if t.len() > 1 && t.starts_with('0') => (&t[1..], 8),
                _ => (t, 10),
            };
            u128::from_str_radix(digits, radix)
                .ok()
                .map(|x| format!("n{x}"))
        }
        _ => None,
    }
}

/// A subject that reads the same value each time: a name or a chain of property reads.
fn plain_subject(n: Node) -> bool {
    match n.kind() {
        "identifier" | "this" => true,
        "field_access" => n.child_by_field_name("object").is_some_and(plain_subject),
        _ => false,
    }
}

/// ContradictoryComparison: in a chain of `||`, two `x != a` with different constants (always
/// true); in a chain of `&&`, two `x == a` (never true). Reported once, on the whole chain.
fn contradictory(src: &str, n: Node, v: &mut Vec<Violation>) {
    let op = operator(n);
    let same_op = |b: Node| b.kind() == "binary_expression" && operator(b) == op;
    if parent_past_parens(n).is_some_and(same_op) {
        return;
    }
    // `x == 1 && (x = 2) && x == 2` can be true
    let mut writes = false;
    crate::preorder(n, |d| {
        writes |= matches!(
            d.kind(),
            "assignment_expression" | "update_expression" | "unary_update_expression"
        );
        !writes
    });
    if writes {
        return;
    }
    let test = if op == "||" { "!=" } else { "==" };
    let mut stack = vec![n];
    let mut seen: Vec<(&str, String)> = Vec::new();
    while let Some(e) = stack.pop() {
        let e = unparen(e);
        if same_op(e) {
            stack.extend(e.child_by_field_name("right"));
            stack.extend(e.child_by_field_name("left"));
            continue;
        }
        if e.kind() != "binary_expression" || operator(e) != test {
            continue;
        }
        let (Some(l), Some(r)) = (
            e.child_by_field_name("left"),
            e.child_by_field_name("right"),
        ) else {
            continue;
        };
        let (subject, value) = match (comparison_constant(src, l), comparison_constant(src, r)) {
            (None, Some(c)) => (unparen(l), c),
            (Some(c), None) => (unparen(r), c),
            _ => continue,
        };
        if !plain_subject(subject) {
            continue;
        }
        let x = text(subject, src);
        // constants of different kinds may both differ from a value of a third kind, so
        // only null mixes with the others
        let kind = |c: &str| c.chars().next();
        if seen.iter().any(|(s, c)| {
            *s == x && *c != value && (kind(c) == kind(&value) || c == "null" || value == "null")
        }) {
            let (what, fix) = if op == "||" {
                ("is true for every value", "&&")
            } else {
                ("is never true", "||")
            };
            v.push(at(
                n,
                "ContradictoryComparison",
                format!("`{x}` can't equal two different constants, so this condition {what}; did you mean {fix}?"),
            ));
            return;
        }
        seen.push((x, value));
    }
}

/// ImpossibleSizeCheck: `x.size() >= 0` or `x.size() < 0` (or `length()`, or 0 on the left)
/// in a condition. An always true one inside `&&`, or a never true one inside `||`, changes
/// nothing (`res.size() >= 0 && res.size() <= 2`, in doris) and is left alone. So are
/// assertions and Spock conditions: of 46 on the corpus, 45 were tests checking on purpose
/// only that a call returns (`result.size() >= 0 // may be 0`); the one `if` was a bug.
fn size_check(src: &str, n: Node, v: &mut Vec<Violation>) {
    let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) else {
        return;
    };
    let is_size = |m: Node| {
        let m = unparen(m);
        m.kind() == "method_invocation"
            && call_args(m).is_some_and(|a| a.named_child_count() == 0)
            && matches!(called_method(src, m), Some("size" | "length"))
    };
    let zero = |z: Node| comparison_constant(src, z).is_some_and(|c| c == "n0");
    // the comparison read with the size on the left
    let op = match (is_size(l) && zero(r), zero(l) && is_size(r), operator(n)) {
        (true, _, op) => op,
        (_, true, "<=") => ">=",
        (_, true, ">") => "<",
        _ => return,
    };
    let always = match op {
        ">=" => true,
        "<" => false,
        _ => return,
    };
    // up through `!`, `&&` and `||` to the statement or expression that branches on it. `!`
    // turns the constant over; an always true part of `&&`, or a never true one of `||`,
    // changes nothing, and is left alone
    let mut c = n;
    let mut value = always;
    let in_condition = loop {
        let Some(p) = c.parent() else { break false };
        match p.kind() {
            "parenthesized_expression" => {}
            "unary_expression" if operator(p) == "!" => value = !value,
            "binary_expression" if matches!(operator(p), "&&" | "||") => {
                if (operator(p) == "&&") == value {
                    return;
                }
            }
            "if_statement" | "while_statement" | "do_while_statement" | "for_statement" => {
                break p.child_by_field_name("condition") == Some(c);
            }
            "ternary_expression" => break p.child_by_field_name("condition") == Some(c),
            "elvis_expression" => break p.child_by_field_name("value") == Some(c),
            _ => break false,
        }
        c = p;
    };
    if !in_condition {
        return;
    }
    v.push(at(
        n,
        "ImpossibleSizeCheck",
        format!(
            "A size is never negative, so `{}` is {}; compare with what was meant (> 0?)",
            text(n, src),
            if always { "always true" } else { "never true" }
        ),
    ));
}

/// CaseConvertedComparison: `s.toLowerCase() == 'Text'` with an uppercase ASCII letter in the
/// string (or `toUpperCase` and a lowercase one), with `==`, `!=` or `equals`. Only ASCII
/// letters: no Unicode case mapping turns a letter into an ASCII one of the other case.
fn case_converted(src: &str, n: Node, v: &mut Vec<Violation>) {
    let (a, b) = if n.kind() == "method_invocation" {
        let arg = call_args(n)
            .filter(|a| a.named_child_count() == 1)
            .and_then(|a| a.named_child(0));
        let object = n
            .child_by_field_name("function")
            .and_then(|f| f.child_by_field_name("object"));
        let (Some(o), Some(arg)) = (object, arg) else {
            return;
        };
        (o, arg)
    } else {
        let (Some(l), Some(r)) = (
            n.child_by_field_name("left"),
            n.child_by_field_name("right"),
        ) else {
            return;
        };
        (l, r)
    };
    let convert = |m: Node| {
        let m = unparen(m);
        if m.kind() != "method_invocation"
            || !call_args(m).is_some_and(|a| a.named_child_count() <= 1)
        {
            return None;
        }
        called_method(src, m).filter(|f| matches!(*f, "toLowerCase" | "toUpperCase"))
    };
    let (method, s) = match (
        convert(a),
        plain_quoted(src, b),
        convert(b),
        plain_quoted(src, a),
    ) {
        (Some(m), Some(s), _, _) | (_, _, Some(m), Some(s)) => (m, s),
        _ => return,
    };
    let (clash, case) = if method == "toLowerCase" {
        (s.chars().any(|c| c.is_ascii_uppercase()), "uppercase")
    } else {
        (s.chars().any(|c| c.is_ascii_lowercase()), "lowercase")
    };
    if clash {
        v.push(at(
            n,
            "CaseConvertedComparison",
            format!("After {method}() the value has no {case} letters, so it never equals '{s}'; write the string in the same case or use equalsIgnoreCase"),
        ));
    }
}

/// JenkinsEnvBooleanComparison: `env.NAME == true`, or `!=`, with either boolean on either
/// side: Jenkins keeps environment variables as strings. A file declaring its own `env`
/// (a variable, field, method, closure or catch parameter, loop variable, or a script
/// binding assigned `env = ...`) is left alone.
fn env_boolean(src: &str, n: Node, v: &mut Vec<Violation>) {
    let (Some(l), Some(r)) = (
        n.child_by_field_name("left"),
        n.child_by_field_name("right"),
    ) else {
        return;
    };
    let env_read = |e: Node| {
        let e = unparen(e);
        e.kind() == "field_access"
            && e.child_by_field_name("object")
                .is_some_and(|o| o.kind() == "identifier" && text(o, src) == "env")
    };
    let boolean = |b: Node| unparen(b).kind() == "boolean_literal";
    let (read, b) = if env_read(l) && boolean(r) {
        (l, r)
    } else if env_read(r) && boolean(l) {
        (r, l)
    } else {
        return;
    };
    let mut root = n;
    while let Some(p) = root.parent() {
        root = p;
    }
    let mut own = false;
    crate::preorder(root, |d| {
        let named = |field: &str| {
            d.child_by_field_name(field)
                .is_some_and(|x| text(x, src) == "env")
        };
        own |= match d.kind() {
            "variable_declarator"
            | "formal_parameter"
            | "closure_parameter"
            | "catch_formal_parameter" => named("name"),
            "for_in_statement" => named("variable"),
            // a script binding, `env = [ok: true]`
            "assignment_expression" => named("left"),
            _ => false,
        };
        !own
    });
    if own {
        return;
    }
    v.push(at(
        n,
        "JenkinsEnvBooleanComparison",
        format!(
            "Jenkins keeps `{}` as a string, so comparing it with {} is always {}; compare with '{}' or use .toBoolean()",
            text(unparen(read), src),
            text(unparen(b), src),
            if operator(n) == "==" { "false" } else { "true" },
            text(unparen(b), src),
        ),
    ));
}

/// UnnecessaryModOne: `x % 1`, which is always 0.
fn mod_one(src: &str, n: Node, v: &mut Vec<Violation>) {
    let one = n
        .child_by_field_name("right")
        .filter(|r| r.kind() == "number_literal")
        .and_then(|r| {
            text(r, src)
                .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                .parse::<f64>()
                .ok()
        })
        == Some(1.0);
    if one && let Some(l) = n.child_by_field_name("left") {
        let l = text(l, src);
        v.push(at(
            n,
            "UnnecessaryModOne",
            format!(
                "{} is guaranteed to be zero. Did you mean ({l} & 1) or ({l} % 2)",
                text(n, src)
            ),
        ));
    }
}

/// IntegerGetInteger: `Integer.getInteger(name)` or `Integer.getInteger(name, default)`,
/// which reads a system property, not a number from a String.
fn integer_get_integer(src: &str, n: Node, v: &mut Vec<Violation>) {
    let on_integer = n
        .child_by_field_name("function")
        .and_then(|f| f.child_by_field_name("object"))
        .is_some_and(|o| text(o, src) == "Integer");
    let args = n.child_by_field_name("arguments").map_or(0, |a| {
        a.named_children(&mut a.walk())
            .filter(|c| !is_comment(*c))
            .count()
    });
    let sig = match args {
        1 => "Integer.getInteger(String)",
        2 => "Integer.getInteger(String, Integer)",
        _ => return,
    };
    if on_integer && n.child_by_field_name("closure").is_none() {
        v.push(at(
            n,
            "IntegerGetInteger",
            format!("{sig} is a confusing API for reading System properties. Prefer the System.getProperty(String) API."),
        ));
    }
}

/// `Math.random()`, without arguments.
fn is_math_random(src: &str, n: Node) -> bool {
    let n = unparen(n);
    n.kind() == "method_invocation"
        && text(n, src).replace(char::is_whitespace, "") == "Math.random()"
}

/// `int` or `Integer` take "an", `long` or `Long` take "a"; `None` for other types.
fn coerced_to_zero(ty: &str) -> Option<&'static str> {
    match ty {
        "int" | "Integer" => Some("an"),
        "long" | "Long" => Some("a"),
        _ => None,
    }
}

/// RandomDoubleCoercedToZero: `(int) Math.random()` or `Math.random() as long`.
fn random_cast(src: &str, n: Node, v: &mut Vec<Violation>) {
    let ty = n.child_by_field_name("type").map_or("", |t| text(t, src));
    if let Some(a) = coerced_to_zero(ty)
        && n.child_by_field_name("value")
            .is_some_and(|x| is_math_random(src, x))
    {
        v.push(at(
            n,
            "RandomDoubleCoercedToZero",
            format!("Casting the result of Math.random() to {a} {ty} always results in 0"),
        ));
    }
}

/// RandomDoubleCoercedToZero: a field declared int, Integer, long or Long whose initial value
/// is `Math.random()`. As in CodeNarc, only fields: a script's or method's local variable
/// isn't checked.
fn random_field(src: &str, n: Node, v: &mut Vec<Violation>) {
    let ty = n.child_by_field_name("type").map_or("", |t| text(t, src));
    let Some(a) = coerced_to_zero(ty) else { return };
    let mut c = n.walk();
    for d in n
        .named_children(&mut c)
        .filter(|d| d.kind() == "variable_declarator")
    {
        if d.child_by_field_name("value")
            .is_some_and(|x| is_math_random(src, x))
        {
            v.push(at(
                n,
                "RandomDoubleCoercedToZero",
                format!("Assigning the result of Math.random() to {a} {ty} always results in 0"),
            ));
        }
    }
}

/// RandomDoubleCoercedToZero: `return Math.random()` (or a ternary or elvis branch of it) in a
/// method declared to return int, Integer, long or Long. A `return` inside a closure in the
/// method returns from the closure, so unlike CodeNarc those aren't counted.
fn random_returned(src: &str, m: Node, v: &mut Vec<Violation>) {
    let ty = m
        .child_by_field_name("return_type")
        .map_or("", |t| text(t, src));
    let Some(a) = coerced_to_zero(ty) else { return };
    let Some(body) = m.child_by_field_name("body") else {
        return;
    };
    crate::preorder(body, |n| match n.kind() {
        "closure" | "lambda_expression" | "class_body" => false,
        "return_statement" => {
            let mut stack: Vec<Node> = n.named_child(0).into_iter().collect();
            while let Some(e) = stack.pop() {
                let e = unparen(e);
                match e.kind() {
                    "ternary_expression" => stack.extend(
                        ["alternative", "consequence"]
                            .iter()
                            .filter_map(|f| e.child_by_field_name(f)),
                    ),
                    "elvis_expression" => stack.extend(
                        ["default", "value"]
                            .iter()
                            .filter_map(|f| e.child_by_field_name(f)),
                    ),
                    _ if is_math_random(src, e) => v.push(at(
                        e,
                        "RandomDoubleCoercedToZero",
                        format!("Returning the result of Math.random() from {a} {ty}-returning method always returns 0"),
                    )),
                    _ => {}
                }
            }
            false
        }
        _ => true,
    });
}

/// ClassNameSameAsFilename: a file whose only top-level code is one class, interface, enum,
/// trait or annotation type, named other than the file (`Foo.groovy` declaring `class Bar`).
/// A file with any other top-level code is a script, and isn't checked.
pub(super) fn class_name_same_as_filename(
    path: &std::path::Path,
    src: &str,
    root: Node,
    v: &mut Vec<Violation>,
) {
    let Some(file) = path
        .file_name()
        .and_then(|f| f.to_str())
        .filter(|f| !f.is_empty())
    else {
        return;
    };
    let mut c = root.walk();
    let mut types = Vec::new();
    for n in root.named_children(&mut c) {
        match n.kind() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "trait_declaration"
            | "annotation_type_declaration"
            | "record_declaration" => types.push(n),
            "package_declaration" | "import_declaration" => {}
            k if k.contains("comment") => {}
            _ => return,
        }
    }
    let [only] = types[..] else { return };
    let Some(name) = only.child_by_field_name("name") else {
        return;
    };
    let shown = text(name, src);
    // CodeNarc compares with the file name less its first `.groovy`
    if shown != file.replacen(".groovy", "", 1) {
        let kind = match only.kind() {
            "interface_declaration" | "annotation_type_declaration" => "Interface",
            "enum_declaration" => "Enum",
            _ => "Class",
        };
        v.push(at(
            name,
            "ClassNameSameAsFilename",
            format!("{kind} `{shown}` is the only class in `{file}`. In such a case the file and the class should have the same name."),
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::lint::lint;

    fn rules(src: &str, rule: &str) -> Vec<usize> {
        lint(src)
            .expect("parses")
            .into_iter()
            .filter(|v| v.rule == rule)
            .map(|v| v.line)
            .collect()
    }

    #[test]
    fn dead_code() {
        let src = "def f() {\n  return 1\n  println 'x'\n}\ndef g(x) {\n  if (x) return 1\n  2\n}\nwhile (a) {\n  break\n  // why\n}\nreturn\ndef h() { }\nclass K { }\nswitch (x) {\n  case 1:\n    throw new E()\n    foo()\n}\nx = { -> return 1\n 2 }\n";
        assert_eq!(rules(src, "DeadCode"), vec![3, 19, 22]);
        // a labeled statement can be reached by a jump
        assert_eq!(
            rules("def f() {\n  return\n  a: f()\n}\n", "DeadCode"),
            vec![] as Vec<usize>
        );
        // `break` after a return in a switch is a habit; code on the jump's line is left to
        // the grammar, which splits `return pipeline { }` (orca DependentPipelineStarterSpec);
        // a doubled `break` has no such reason and is reported
        let src = "switch (x) {\n  case 1:\n    return 1\n    break\n  case 2:\n    throw new E()\n    break\n}\nx = { _, p ->\n  return pipeline {\n    name = p\n  }\n}\nwhile (a) {\n  break\n  break\n}\n";
        assert_eq!(rules(src, "DeadCode"), vec![16]);
    }

    /// #212: Groovy 4's `String.leftShift` returns a new StringBuffer.
    #[test]
    fn string_left_shift_discarded() {
        let rule = "StringLeftShiftDiscarded";
        // in a loop, an if in a loop, a chain, a .each { }, a void method, a String type
        let src = "void f(cs) {\n  def s = ''\n  for (c in cs) {\n    if (!c) {\n      s << \"x $c\"\n    } else {\n      s << c << '\\n'\n    }\n  }\n  cs.each { s << it }\n  String t = \"a\"\n  t << 'b'\n  def u = 'a'\n  u = 'b'\n  u << 1\n  println s + t + u\n}\n";
        assert_eq!(rules(src, rule), vec![5, 7, 10, 12, 15]);
        let src = "def f(cs) {\n  def a = ''\n  def c = { a << 1 }\n  def b = ''\n  b = b << 1\n  b << 2\n  def d = new StringBuilder()\n  d << 1\n  def e = 'x'\n  cs.each { e -> e << 1; println e }\n  def g = 'x'\n  try { } catch (g2) { }\n  x << 1\n  def h = [], k\n  h << 1\n  def (m, n) = ['', '']\n  m << 1\n  StringBuilder p = ''\n  p << 1\n  def q = ''\n  q <<= 1\n  q << 2\n  def gs = \"${x}\"\n  gs << 1\n  def w = ''\n  w = \"$x\"\n  w << 1\n  def r = ''\n  r << 1\n}\n";
        assert_eq!(rules(src, rule), Vec::<usize>::new());
        // a field, a closure's or a method's result, a script's last statement (before its
        // methods and classes)
        let src = "class K {\n  String s = ''\n  void f() {\n    s << 1\n  }\n  String g() {\n    def t = ''\n    t << 1\n  }\n}\ndef u = ''\nu << 1\n";
        assert_eq!(rules(src, rule), Vec::<usize>::new());
        let src = "def u = ''\nu << 1 // result\n/* end */\nvoid f() { }\nclass K { }\n";
        assert_eq!(rules(src, rule), Vec::<usize>::new());
        // #230: a script's top-level statement that isn't its last; not a binding variable,
        // or a script method's own
        let src = "def s = ''\ns << 'x'\nString t = 'a'\nt << 'b'\nb = ''\nb << 1\nvoid f() { s << 1 }\nprintln s + t\n";
        assert_eq!(rules(src, rule), vec![2, 4]);
        assert_eq!(
            msgs(src, rule)[0],
            "`s` is a String: `<<` returns a new StringBuffer and leaves `s` unchanged, so the statement has no effect. To collect into `s`, make it a StringBuilder or write `s += ...`, and check what the code reading `s` expects"
        );
    }

    #[test]
    fn finally_blocks() {
        let src = "try { f() } finally {\n  return 1\n}\ntry { f() } finally {\n  throw new E()\n}\ntry { f() } finally {\n  assert x\n}\ntry { f() } finally {\n  xs.each { return it }\n  try { throw new E() } catch (e) { }\n  def g = { assert x }\n}\n";
        assert_eq!(rules(src, "ReturnFromFinallyBlock"), vec![2]);
        assert_eq!(rules(src, "ThrowExceptionFromFinallyBlock"), vec![5]);
        assert_eq!(rules(src, "AssertWithinFinallyBlock"), vec![8]);
    }

    #[test]
    fn empty_switch_and_initializers() {
        let src = "switch (x) { }\nswitch (x) {\n  // nothing yet\n}\nclass A {\n  static { }\n  { }\n  static { f() }\n  { g() }\n}\n";
        assert_eq!(rules(src, "EmptySwitchStatement"), vec![1]);
        assert_eq!(rules(src, "EmptyStaticInitializer"), vec![6]);
        assert_eq!(rules(src, "EmptyInstanceInitializer"), vec![7]);
    }

    #[test]
    fn set_values() {
        let src = "def a = [1, 2, 1] as Set\ndef b = ['a', \"a\"] as HashSet\ndef c = [1, '1', 1L, 1.0] as Set\ndef d = [1, 1] as List\ndef e = [x, x] as Set\ndef f = [\"${x}\", \"${x}\"] as Set\n";
        assert_eq!(rules(src, "DuplicateSetValue"), vec![1, 2]);
    }

    #[test]
    fn constant_conditions() {
        let src = "def a = true ? 1 : 2\ndef b = 'x' ?: 2\ndef c = [:] ? 1 : 2\ndef d = Boolean.TRUE ? 1 : 2\ndef e = x ? 1 : 2\ndef f = [*xs] ? 1 : 2\ndef g = \"${x}\" ?: 2\nassert true\nassert x\nassert null, 'm'\n";
        assert_eq!(rules(src, "ConstantTernaryExpression"), vec![1, 2, 3, 4]);
        assert_eq!(rules(src, "ConstantAssertExpression"), vec![8, 10]);
    }

    #[test]
    fn comparisons_of_constants() {
        let src = "if (1 == 2) { f() }\nx = 'a' < 'b'\ny = [1, [a: true]] != [2]\nz = x == 1\nw = [x] == [1]\nv = Boolean.FALSE != false\nu = \"${a}\" == 'a'\nt = [a: 1] <=> [a: 2]\ns = [1].equals([2])\nr = 'a'.compareTo('b')\nq = 'a'.equals(x)\n";
        assert_eq!(
            rules(src, "ComparisonOfTwoConstants"),
            vec![1, 2, 3, 6, 8, 9, 10]
        );
    }

    #[test]
    fn unintended_regex_patterns() {
        let src = "a = s.split('.')\nb = s.split(\"|\", 2)\nc = s.replaceAll('.', '-')\nd = s.replaceFirst('|', '')\ne = s.split('\\\\.')\nf = s.split(/./)\ng = s.replaceAll('.', ' ')\nh = s.replaceAll('.', '=')\ni = s.replaceFirst('.') { it.toUpperCase() }\nj = s.tokenize('.')\nk = s.replace('.', '-')\nl = s.split(',')\nm = s.replaceAll('|', ' ')\nn = s.replaceAll('.', sep)\no = Pattern.compile(',').split('.')\np = Splitter.on(',').split('|')\nq = s.replaceAll('.', 'X')\nr = s.replaceAll('.', '_')\nt = s.replaceAll('.', '')\n";
        assert_eq!(
            rules(src, "UnintendedRegexPattern"),
            vec![1, 2, 3, 4, 13, 18, 19]
        );
    }

    #[test]
    fn contradictory_comparisons() {
        let src = "if (m != 'heat' || m != 'cool') { f() }\nif (a.t == 1 && a.t == 0xFF) { f() }\nif (x != null || x != '') { f() }\nif (ok && (x == 1 && x == 2)) { f() }\nif (m != 'heat' && m != 'cool') { f() }\nif (x == 1 || x == 2) { f() }\nif (x != 1 || x != '1') { f() }\nif (x == 1 && x == 1L) { f() }\nif (x == 1 && x == 1.0) { f() }\nif (n() != 1 || n() != 2) { f() }\nif (x != 1 || y != 2) { f() }\nif (x != 1 || b || 3 != x) { f() }\n";
        assert_eq!(rules(src, "ContradictoryComparison"), vec![1, 2, 3, 4, 12]);
        let src =
            "if (x == 1 && (x = 2) && x == 2) { f() }\nif (x == 1 && x++ && x == 2) { f() }\n";
        assert!(rules(src, "ContradictoryComparison").is_empty());
    }

    #[test]
    fn impossible_size_checks() {
        let src = "if (xs.size() >= 0) { f() }\nif (s.length() < 0) { f() }\nwhile (0 <= xs.size()) { f() }\nif (0 > xs.size()) { f() }\nif (xs.size() >= 0 && xs.size() <= 2) { f() }\nif (xs.size() < 0 || ok) { f() }\nif (xs.size() > 0) { f() }\nif (xs.size >= 0) { f() }\nif (xs.size(1) >= 0) { f() }\nif (ok && (xs.size() >= 0 || ok)) { f() }\nr = !(xs.size() < 0) ? 1 : 2\nassert xs.size() >= 0\ndef b = xs.size() >= 0\nassertTrue(xs.size() >= 0)\nif (f(xs.size() >= 0)) { f() }\ndo { f() } while (xs.size() >= 0)\nif (!(xs.size() < 0) && ok) { f() }\nif (!(xs.size() >= 0) || ok) { f() }\nif ((xs.size() >= 0) == flag) { f() }\nif (xs.size() >= 0L) { f() }\nif (!(xs.size() < 0) || ok) { f() }\n";
        assert_eq!(
            rules(src, "ImpossibleSizeCheck"),
            vec![1, 2, 3, 4, 11, 16, 20, 21]
        );
    }

    #[test]
    fn case_converted_comparisons() {
        let src = "a = m.toLowerCase() == 'Automatic'\nb = 'abc' != s.toUpperCase()\nc = s.toLowerCase(Locale.ROOT).equals('X')\nd = m.toLowerCase() == 'automatic'\ne = s.toLowerCase() == 'Run'.toLowerCase()\nf = s.toUpperCase() == 'ABC_1'\ng = s.toLowerCase() == \"${X}\"\nh = s.toLowerCase() == 'caf\\u00C9'\n";
        assert_eq!(rules(src, "CaseConvertedComparison"), vec![1, 2, 3]);
    }

    #[test]
    fn env_boolean_comparisons() {
        let src = "if (env.IS_RELEASE == true) { f() }\nif (false != env.DRY) { f() }\nif (env.IS_RELEASE == 'true') { f() }\nif (params.X == true) { f() }\n";
        assert_eq!(rules(src, "JenkinsEnvBooleanComparison"), vec![1, 2]);
        let own = "def check(env) { env.ok == true }\n";
        assert!(rules(own, "JenkinsEnvBooleanComparison").is_empty());
        for own in [
            "def env = [ok: true]\nassert env.ok == true\n",
            "for (env in envs) { if (env.ok == true) f() }\n",
            "for (Map env : envs) { if (env.ok == true) f() }\n",
            "try { } catch (Exception env) { if (env.ok == true) f() }\n",
            "xs.each { env -> if (env.ok == true) f() }\n",
            "xs.each { Map env -> if (env.ok == true) f() }\n",
            "env = [ok: true]\nassert env.ok == true\n",
        ] {
            assert!(
                rules(own, "JenkinsEnvBooleanComparison").is_empty(),
                "{own}"
            );
        }
    }

    #[test]
    fn null_and_oddness_checks() {
        let src = "if (x != null || x.size() > 0) { f() }\nif (x == null && x.a) { f() }\nif (r == null && !r.id) { f() }\nif (x != null && x.size() > 0) { f() }\nif (x == null || x.a) { f() }\nif (x != null || y.a) { f() }\nif (x != null || x?.a) { f() }\nif (this.a != null || this.a.b) { f() }\n";
        assert_eq!(rules(src, "BrokenNullCheck"), vec![1, 2, 3, 8]);
        let src = "if (x % 2 == 1) { f() }\nif (1 == f() % 2) { f() }\nif (x % 2 != 0) { f() }\nif (x % 2 == 0) { f() }\nif (x % 3 == 1) { f() }\n";
        assert_eq!(rules(src, "BrokenOddnessCheck"), vec![1, 2]);
    }

    #[test]
    fn remove_all_on_self() {
        let src = "a.removeAll(a)\nthis.a.removeAll(this.a)\na.removeAll(b)\nf().removeAll(f())\n";
        assert_eq!(rules(src, "RemoveAllOnSelf"), vec![1, 2]);
    }

    #[test]
    fn equals_and_hash_code() {
        let src = "class A {\n  boolean equals(Object o) { true }\n}\nclass B {\n  int hashCode() { 1 }\n}\nclass C {\n  boolean equals(o) { true }\n  int hashCode() { 1 }\n}\n@EqualsAndHashCode\nclass D {\n  boolean equals(Object o) { true }\n}\nclass E {\n  boolean equals(String s) { true }\n  static int hashCode() { 1 }\n}\nclass F extends A {\n  boolean equals(Object o) { true }\n}\n";
        assert_eq!(rules(src, "EqualsAndHashCode"), vec![1]);
        assert_eq!(rules(src, "EqualsOverloaded"), vec![16]);
        // #212: an @Override of a base class's typed equals is not an overload by mistake
        let src = "class A extends B {\n  @Override\n  protected boolean equals(Jwk o) { true }\n  @java.lang.Override boolean equals(A o) { true }\n  @Deprecated boolean equals(C o) { true }\n}\n";
        assert_eq!(rules(src, "EqualsOverloaded"), vec![5]);
        // hoist-core's Filter: equals(Object) delegates to a typed overload, abstract in the
        // base; a superclass in another file can't be seen, so Lone stays reported
        let src = "abstract class Filter {\n  abstract boolean equals(Filter o)\n  boolean equals(Object o) { o instanceof Filter && equals((Filter) o) }\n}\nclass Field extends Filter {\n  boolean equals(Filter o) { true }\n  int hashCode() { 1 }\n}\nclass Other {\n  boolean equals(Other o) { true }\n  boolean equals(Object o) { o instanceof Other && equals((Other) o) }\n  int hashCode() { 1 }\n}\nclass Lone extends Remote {\n  boolean equals(Lone o) { true }\n}\nabstract class Base {\n  int x\n  abstract int size()\n  boolean equals(Object o) { o.x == x }\n}\n";
        assert_eq!(rules(src, "EqualsOverloaded"), vec![15]);
        assert_eq!(rules(src, "EqualsAndHashCode"), vec![17]);
        // an equals never broader than identity keeps the contract with Object.hashCode
        // (grails-core's VersionComparator and NullSafeNavigator)
        let src = "class A {\n  boolean equals(obj) { false }\n}\nclass B {\n  boolean equals(Object to) {\n    return to == null || DefaultGroovyMethods.is(this, to)\n  }\n}\nclass C {\n  boolean equals(Object o) { this.is(o) }\n}\nclass D {\n  boolean equals(Object o) { true }\n}\nclass E {\n  boolean equals(Object o) { o == null || o.x == 1 }\n}\n";
        assert_eq!(rules(src, "EqualsAndHashCode"), vec![12, 15]);
    }

    #[test]
    fn object_methods() {
        let src = "class A {\n  boolean equal(Object o) { true }\n  boolean equal(int o) { true }\n  boolean equaLS(o) { true }\n  int hashcode() { 1 }\n  int hashcode(int v) { 1 }\n  String toSTring() { '' }\n  String tostring(int v) { '' }\n  boolean equals(Object o, int x) { true }\n  boolean equals() { true }\n  static boolean equals(a, b) { true }\n}\n";
        assert_eq!(
            rules(src, "ObjectOverrideMisspelledMethodName"),
            vec![2, 4, 5, 7]
        );
        assert_eq!(rules(src, "EqualsOverloaded"), vec![] as Vec<usize>);
        // the grammar reads a method with modifiers and no return type as a constructor
        let src = "class A {\n  private tostring() { '' }\n  A() { }\n}\n";
        assert_eq!(rules(src, "ObjectOverrideMisspelledMethodName"), vec![2]);
    }

    #[test]
    fn to_string_returns_null() {
        let src = "class A {\n  String toString() {\n    if (x) {\n      return 'a'\n    }\n    return null\n  }\n}\nclass B {\n  String toString() {\n    f()\n    null\n  }\n}\nclass C {\n  String toString() { }\n}\nclass D {\n  String toString() { xs.find { return null } ?: 'd' }\n  String toString(int i) { null }\n}\n";
        assert_eq!(rules(src, "ToStringReturnsNull"), vec![6, 12, 16]);
    }

    #[test]
    fn throws() {
        let src = "throw RuntimeException('x')\nthrow new RuntimeException('x')\nthrow runtimeException()\nthrow Fault()\nthrow a.BadException()\nthrow IOException\nthrow e\n";
        assert_eq!(rules(src, "MissingNewInThrowStatement"), vec![1, 4, 6]);
        let src = "void f() {\n  try { g() } catch (e) { new IllegalStateException(e) }\n}\ndef h() {\n  try { g() } catch (e) {\n    new IllegalStateException(e)\n  }\n}\nvoid k() {\n  try { g() } catch (e) {\n    throw new IllegalStateException(e)\n  }\n  try { g() } catch (e) {\n    new IllegalStateException(e)\n  }\n  done()\n}\nx = { try { g() } catch (e) { new MyException() } }\n";
        assert_eq!(rules(src, "ExceptionNotThrown"), vec![2, 14]);
    }

    // The cases below for the rules from CodeNarc's basic, unnecessary and naming rulesets are
    // ported from CodeNarc's own tests (org.codenarc.rule.basic.*RuleTest and so on,
    // Copyright Chris Mair and the CodeNarc contributors, Apache License 2.0).

    fn msgs(src: &str, rule: &str) -> Vec<String> {
        lint(src)
            .expect("parses")
            .into_iter()
            .filter(|v| v.rule == rule)
            .map(|v| v.message)
            .collect()
    }

    #[test]
    fn big_decimal_instantiation() {
        let src = "class MyClass {\n  def b1 = new BigDecimal(123.45)\n  def b2 = new java.math.BigDecimal(0.26789d)\n  def b3 = new BigDecimal(123.45, MathContext.UNLIMITED)\n  def c = { def b4 = new BigDecimal(12.0001) }\n}\ndef b5 = new BigDecimal(12345678.987654321d)\ndef b6 = new BigDecimal(1e3)\n";
        assert_eq!(
            rules(src, "BigDecimalInstantiation"),
            vec![2, 3, 4, 5, 7, 8]
        );
        let ok = "def a = new BigDecimal(BigInteger.ONE)\ndef b = new BigDecimal(BigInteger.TEN, 3)\ndef c = new BigDecimal('123.45' as char[])\ndef d = new BigDecimal(99)\ndef e = new BigDecimal(12345, MathContext.UNLIMITED)\ndef f = new BigDecimal(99L)\ndef g = new BigDecimal(L, MathContext.UNLIMITED)\ndef h = new BigDecimal('12345.67')\ndef i = new BigDecimal(xyz)\ndef j = new BigDecimal(1.5f)\ndef k = new BigInteger(1.5)\n";
        assert!(rules(ok, "BigDecimalInstantiation").is_empty());
        assert_eq!(
            msgs("x = new BigDecimal(12.0001)\n", "BigDecimalInstantiation")[0],
            "Call to new BigDecimal(12.0001) uses the double constructor and should probably be replaced with new BigDecimal(\"12.0001\")"
        );
    }

    #[test]
    fn bitwise_operator_in_conditional() {
        let ok = "def temp = (3 | 6)\nif (temp == 7) { return true }\nif (a || b) { return true }\nwhile (temp == 7) { }\nif (!(a & b)) { }\nif (f(a | b)) { }\nfor (; a | b;) { }\n";
        assert!(rules(ok, "BitwiseOperatorInConditional").is_empty());
        let src = "if (a|b) { return true }\nif (a|b||a) { return true }\nif ((a|b)||(a||b)) { }\nif ((a||b)||(a|b)) { }\nif ((a||b)||a && b || b || (a|b)) { }\nwhile (a|b) { }\na|b ? true : false\na|b ?: false\nif (a&b) { }\nif ((flags & MASK) != 0) { }\nif (a | b && c & d) { }\n";
        assert_eq!(
            rules(src, "BitwiseOperatorInConditional"),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 11]
        );
    }

    #[test]
    fn groovy24_empty_element() {
        let ok = "foo(a, b)\ndef l = [a, b,]\ndef m = [a: 1, b: 2]\n";
        assert!(rules(ok, "Groovy24EmptyElement").is_empty());
        let src = "foo(a,,b)\ndef l = [\"off\", , \"heat\"]\ndef m = [a: 1,, b: 2]\nfoo(a,,, b)\nfoo(a, /* x */ , b)\n";
        assert_eq!(rules(src, "Groovy24EmptyElement"), vec![1, 2, 3, 4, 4, 5]);
        assert!(msgs("foo(a,,b)\n", "Groovy24EmptyElement")[0].starts_with("An empty argument"));
        assert!(
            msgs("def m = [a: 1,, b: 2]\n", "Groovy24EmptyElement")[0]
                .starts_with("An empty map entry")
        );
    }

    #[test]
    fn double_negative() {
        assert!(rules("!true\ndef x = !a.b\n", "DoubleNegative").is_empty());
        let src = "def x = !!true\ndef y = !(!true)\n";
        assert_eq!(rules(src, "DoubleNegative"), vec![1, 2]);
    }

    #[test]
    fn empty_synchronized_statement() {
        let src = "class MyClass {\n  def myClosure = {\n    synchronized(lock) {\n    }\n  }\n}\nsynchronized(lock) {\n  // waits for the writer\n}\nsynchronized(lock) {\n  println 'x'\n}\n";
        // a comment counts as content, as for the other Empty* rules
        assert_eq!(rules(src, "EmptySynchronizedStatement"), vec![3]);
    }

    #[test]
    fn for_loop_should_be_while_loop() {
        let ok = "while (1 > 2)\n  println 'never'\nfor (i in [1, 2])\n  println i\nfor (child in this.children) { }\nfor (int i = 0; i < 5;)\n  println i++\nfor (; i < 5; i++)\n  println i\nfor (Plan p : plans) { }\nfor (;;) { break }\n";
        assert!(rules(ok, "ForLoopShouldBeWhileLoop").is_empty());
        assert_eq!(
            rules(
                "int i = 0\nfor (; i < 5;)\n  println i++\n",
                "ForLoopShouldBeWhileLoop"
            ),
            vec![2]
        );
    }

    #[test]
    fn integer_get_integer() {
        let ok = "Integer.getInteger()\nInteger.getInteger(value, radix, locale)\nInteger.parseInt(value)\nfoo.getInteger(value)\n";
        assert!(rules(ok, "IntegerGetInteger").is_empty());
        let src = "Integer.getInteger(value)\nInteger.getInteger(value, radix)\n";
        assert_eq!(rules(src, "IntegerGetInteger"), vec![1, 2]);
    }

    #[test]
    fn random_double_coerced_to_zero() {
        let ok = "class MyClass {\n  def a = (double) Math.random()\n  def b = (Double) Math.random()\n  double c = Math.random()\n  Double d = Math.random()\n  def e = (Math.random()) as double\n  def f = (Math.random()) as Double\n  def g = Math.random()\n}\nDouble intMethod() {\n  if (foo) return Math.random()\n}\ndouble integerMethod() {\n  if (foo) return Math.random()\n}\ndef defMethod() {\n  if (foo) return Math.random()\n}\nint h() {\n  [1].collect { return Math.random() }\n}\nlong local = Math.random()\n";
        assert!(rules(ok, "RandomDoubleCoercedToZero").is_empty());
        let src = "class MyClass {\n  def a = (long) Math.random()\n  def b = (Long) Math.random()\n  long c = Math.random()\n  Long d = Math.random()\n  def e = (Math.random()) as long\n  def f = (Math.random()) as Long\n  def g = (int) Math.random()\n  def h = (Integer) Math.random()\n  int i = Math.random()\n  Integer j = Math.random()\n  def k = (Math.random()) as int\n}\nlong longMethod() {\n  if (foo) return Math.random()\n}\nLong longMethod2() {\n  return ((foo) ?: Math.random())\n}\nint intMethod() {\n  return foo ? 1 : Math.random()\n}\n";
        assert_eq!(
            rules(src, "RandomDoubleCoercedToZero"),
            vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 18, 21]
        );
        let v = msgs(
            "class A {\n  long c = Math.random()\n  int f() { return Math.random() }\n}\n",
            "RandomDoubleCoercedToZero",
        );
        assert_eq!(
            v[0],
            "Assigning the result of Math.random() to a long always results in 0"
        );
        assert_eq!(
            v[1],
            "Returning the result of Math.random() from an int-returning method always returns 0"
        );
    }

    #[test]
    fn unnecessary_instance_of_check() {
        let ok = "if (!(variable instanceof String)) { }\ndef x = !(variable instanceof String)\ndef y = variable instanceof String\n";
        assert!(rules(ok, "UnnecessaryInstanceOfCheck").is_empty());
        let src = "def x = !variable instanceof String\ndef y = !variable instanceof Boolean\nif (!var instanceof Integer) { }\n";
        assert_eq!(rules(src, "UnnecessaryInstanceOfCheck"), vec![1, 2, 3]);
        let v = msgs(src, "UnnecessaryInstanceOfCheck");
        assert_eq!(v[0], "The result of '!variable' will never be a String");
        assert_eq!(v[1], "The result of '!variable' will always be a Boolean");
    }

    /// #230: `!x in xs` is `(!x) in xs` in Groovy 2.4, 3 and 4.
    #[test]
    fn negated_in_check() {
        let rule = "NegatedInCheck";
        let ok = "if (!(name in xs)) { }\ndef a = name !in xs\ndef b = !name\ndef c = (!name) in [true]\ndef d = !(a.b in xs) && !c\nfor (x in xs) { }\n";
        assert!(rules(ok, rule).is_empty());
        let src = "def a = !name in ['a', 'b']\nif (!x.simpleName in ['JpaSession']) { }\nassert !f(x) !in xs\n";
        assert_eq!(rules(src, rule), vec![1, 2, 3]);
        let v = msgs(src, rule);
        assert_eq!(
            v[0],
            "`!` applies to `name` alone, so this asks whether the boolean `!name` is a member; write `!(name in ...)`"
        );
        assert!(v[2].ends_with("write `!(f(x) !in ...)`"), "{}", v[2]);
    }

    #[test]
    fn unnecessary_mod_one() {
        assert!(rules("if (exp & 1) { }\nif (exp % 2) { }\n", "UnnecessaryModOne").is_empty());
        let src = "if (exp % 1) { }\nif (method() % 1) { }\nx = y % 1L\n";
        assert_eq!(rules(src, "UnnecessaryModOne"), vec![1, 2, 3]);
        assert_eq!(
            msgs("x = exp % 1\n", "UnnecessaryModOne")[0],
            "exp % 1 is guaranteed to be zero. Did you mean (exp & 1) or (exp % 2)"
        );
    }

    #[test]
    fn class_name_same_as_filename() {
        use std::path::Path;
        let at = |name: &str, src: &str| -> Vec<usize> {
            crate::lint::lint_file(Path::new(name), src)
                .unwrap()
                .into_iter()
                .filter(|v| v.rule == "ClassNameSameAsFilename")
                .map(|v| v.line)
                .collect()
        };
        let none: Vec<usize> = vec![];
        assert_eq!(
            at(
                "SameAsFilename.groovy",
                "class SameAsFilename {\n  static class Foo {}\n}\n"
            ),
            none
        );
        assert_eq!(at("", "class Ignore { }\n"), none);
        assert_eq!(
            at(
                "SameAsFilename.groovy",
                "class SameAsFilename {}\nenum NotSameAsFilename {}\n"
            ),
            none
        );
        assert_eq!(
            at("SameAsFilename.groovy", "println 'Hello world!'\n"),
            none
        );
        assert_eq!(
            at(
                "SameAsFilename.groovy",
                "package a\nimport b.C\n\nclass NotSameAsFilename {}\n"
            ),
            vec![4]
        );
        assert_eq!(
            at("SameAsFilename.groovy", "\nenum NotSameAsFilename {}\n"),
            vec![2]
        );
        assert_eq!(
            at(
                "SameAsFilename.groovy",
                "\ninterface NotSameAsFilename {}\n"
            ),
            vec![2]
        );
        assert_eq!(
            at(
                "SameAsFilename.groovy",
                "class NotSameAsFilename {\n  enum Foo {}\n  class Baz {}\n}\n"
            ),
            vec![1]
        );
        // a script with a class in it is a script
        assert_eq!(
            at("SameAsFilename.groovy", "class A {}\nprintln new A()\n"),
            none
        );
        let v = crate::lint::lint_file(Path::new("src/X.groovy"), "class Y {}\n").unwrap();
        assert_eq!(
            v[0].message,
            "Class `Y` is the only class in `X.groovy`. In such a case the file and the class should have the same name."
        );
    }
}
