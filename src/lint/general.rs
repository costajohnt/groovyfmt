//! The rules on plain Groovy that need no module of their own: imports, empty blocks,
//! conditions, duplicate keys and cases, GStrings and semicolons, and the helpers on
//! imports and GStrings other modules and `--fix` share.

use super::{Edit, Rule, Violation, at, own_lines, unused};
use crate::text;
use std::collections::HashSet;
use tree_sitter::{Node, Tree};

pub(super) const RULES: &[Rule] = &[
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
];

/// This module's rules on node `n`, whose kind is `kind`, in `lint_with`'s preorder walk:
/// `rest` is the nodes after it.
pub(super) fn node(src: &str, kind: &str, n: Node, rest: &[Node], v: &mut Vec<Violation>) {
    match kind {
        "string_literal" => unnecessary_gstring(src, n, v),
        ";" => unnecessary_semicolon(src, n, rest, v),
        "catch_clause" => empty_catch(src, n, v),
        "if_statement" => if_statement(n, v),
        "while_statement" if !condition_works(n) => {
            empty_body(
                n,
                "body",
                "EmptyWhileStatement",
                "The while statement is empty",
                v,
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
            v,
        ),
        "for_in_statement" if iterates_literal(n) => empty_body(
            n,
            "body",
            "EmptyForStatement",
            "The for statement is empty",
            v,
        ),
        "try_statement" => empty_body(n, "body", "EmptyTryBlock", "The try block is empty", v),
        "finally_clause" => empty_body(
            n,
            "body",
            "EmptyFinallyBlock",
            "The finally block is empty",
            v,
        ),
        "map_literal" => duplicate_map_key(src, n, v),
        "switch_block" => duplicate_case(src, n, v),
        "binary_expression" => comparison_with_self(src, n, v),
        _ => {}
    }
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
pub(super) fn imports(src: &str, tree: &Tree, v: &mut Vec<Violation>) {
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
pub(super) fn words(s: &str) -> impl Iterator<Item = &str> {
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

pub(super) fn is_empty_block(n: Option<Node>) -> bool {
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

pub(super) fn is_literal(n: Node) -> bool {
    matches!(
        n.kind(),
        "boolean_literal" | "null_literal" | "number_literal" | "string_literal"
    ) && !has_interpolation(n)
}

pub(super) fn has_interpolation(n: Node) -> bool {
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
pub(super) fn constant_value(n: Node, src: &str) -> Option<String> {
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
