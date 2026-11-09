//! Native lint rules, named after their CodeNarc equivalents so CI configs map across.
//! Files with parse errors are not linted (a broken tree would produce false positives).

use std::collections::HashSet;
use tree_sitter::{Node, Tree};

pub struct Rule {
    pub name: &'static str,
    pub default: bool,
    pub description: &'static str,
}

/// Every rule, named after its CodeNarc equivalent so existing configs map across.
pub const RULES: &[Rule] = &[
    Rule {
        name: "UnusedImport",
        default: true,
        description: "import whose name (or alias) never appears in code or comments",
    },
    Rule {
        name: "DuplicateImport",
        default: true,
        description: "the same import twice",
    },
    Rule {
        name: "UnnecessarySemicolon",
        default: true,
        description: "`;` at the end of a line",
    },
    Rule {
        name: "EmptyCatchBlock",
        default: true,
        description: "empty catch, unless the parameter is named ignore / ignored",
    },
    Rule {
        name: "EmptyIfStatement",
        default: true,
        description: "if statement with an empty block",
    },
    Rule {
        name: "EmptyElseBlock",
        default: true,
        description: "else with an empty block",
    },
    Rule {
        name: "EmptyTryBlock",
        default: true,
        description: "try with an empty block",
    },
    Rule {
        name: "EmptyFinallyBlock",
        default: true,
        description: "finally with an empty block",
    },
    Rule {
        name: "EmptyWhileStatement",
        default: true,
        description: "while loop with an empty block",
    },
    Rule {
        name: "EmptyForStatement",
        default: true,
        description: "for loop with an empty block",
    },
    Rule {
        name: "DuplicateMapKey",
        default: true,
        description: "a map literal with the same constant key twice; the later entry wins",
    },
    Rule {
        name: "DuplicateCaseStatement",
        default: true,
        description: "a switch with two case labels for the same constant",
    },
    Rule {
        name: "AssignmentInConditional",
        default: true,
        description: "`=` where `==` was probably meant, in an if condition (`while (x = next())` is a common idiom, so loops are exempt)",
    },
    Rule {
        name: "ComparisonWithSelf",
        default: false,
        description: "a variable compared with itself (off by default: GORM `where { title == title }` and Spock equality checks do this on purpose)",
    },
    Rule {
        name: "ConstantIfExpression",
        default: true,
        description: "an if condition that is a literal, so one branch never runs",
    },
    Rule {
        name: "UnnecessaryGString",
        default: false,
        description: "double-quoted string with nothing to interpolate (off by default: noisy where double quotes are the house style)",
    },
];

#[derive(Debug, PartialEq)]
pub struct Violation {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based
    pub rule: &'static str,
    pub message: String,
}

pub fn lint(src: &str) -> Result<Vec<Violation>, &'static str> {
    let tree = crate::parse(src);
    if tree.root_node().has_error() {
        return Err("parse error");
    }
    let mut v = Vec::new();
    imports(src, &tree, &mut v);
    let mut nodes = Vec::new();
    each(tree.root_node(), &mut |n| nodes.push(n));
    for (i, &n) in nodes.iter().enumerate() {
        match n.kind() {
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
            _ => {}
        }
    }
    v.sort_by_key(|x| (x.line, x.col));
    Ok(v)
}

fn each<'a>(n: Node<'a>, f: &mut impl FnMut(Node<'a>)) {
    f(n);
    let mut c = n.walk();
    for child in n.children(&mut c) {
        each(child, f);
    }
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    n.utf8_text(src.as_bytes()).unwrap_or("")
}

fn at(n: Node, rule: &'static str, message: String) -> Violation {
    let p = n.start_position();
    Violation {
        line: p.row + 1,
        col: p.column + 1,
        rule,
        message,
    }
}

/// UnusedImport + DuplicateImport.
fn imports(src: &str, tree: &Tree, v: &mut Vec<Violation>) {
    fn scan<'a>(
        n: Node<'a>,
        src: &'a str,
        decls: &mut Vec<Node<'a>>,
        used: &mut HashSet<&'a str>,
        comments: &mut String,
    ) {
        match n.kind() {
            "import_declaration" => decls.push(n), // don't descend: names inside imports aren't uses
            k if k.contains("comment") => {
                comments.push_str(text(n, src));
                comments.push('\n');
            }
            _ if n.child_count() == 0 => {
                used.insert(text(n, src));
            }
            _ => {
                let mut c = n.walk();
                for child in n.children(&mut c) {
                    scan(child, src, decls, used, comments);
                }
            }
        }
    }
    let (mut decls, mut used, mut comments) = (Vec::new(), HashSet::new(), String::new());
    scan(tree.root_node(), src, &mut decls, &mut used, &mut comments);
    let mut seen = HashSet::new();
    for d in decls {
        let full = text(d, src)
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !seen.insert(full.clone()) {
            v.push(at(
                d,
                "DuplicateImport",
                format!("Duplicate import: {full}"),
            ));
            continue;
        }
        let name = d.child_by_field_name("name").map_or("", |n| text(n, src));
        if name.ends_with('*') || full.ends_with(".*") {
            continue;
        }
        let symbol = d
            .child_by_field_name("alias")
            .map_or_else(|| name.rsplit('.').next().unwrap_or(name), |a| text(a, src));
        // ponytail: a mention in a comment ({@link Foo}) counts as use, as CodeNarc does; no word-boundary check.
        if !symbol.is_empty() && !used.contains(symbol) && !comments.contains(symbol) {
            v.push(at(
                d,
                "UnusedImport",
                format!("The [{name}] import is never referenced"),
            ));
        }
    }
}

/// UnnecessaryGString: double-quoted string with nothing to interpolate.
fn unnecessary_gstring(src: &str, n: Node, v: &mut Vec<Violation>) {
    let t = text(n, src);
    let mut c = n.walk();
    let interpolates = n
        .named_children(&mut c)
        .any(|k| k.kind().starts_with("gstring_"));
    if t.starts_with('"') && !interpolates && !t.contains('\'') && !t.contains('$') {
        v.push(at(
            n,
            "UnnecessaryGString",
            format!("The String {t} can be wrapped in single quotes instead of double quotes"),
        ));
    }
}

/// UnnecessarySemicolon: a `;` that ends a line (or the file).
fn unnecessary_semicolon(n: Node, rest: &[Node], v: &mut Vec<Violation>) {
    if n.parent()
        .is_some_and(|p| p.kind().contains("enum") || p.kind() == "for_statement")
    {
        return;
    }
    let row = n.start_position().row;
    // next leaf in document order, ignoring comments
    let next = rest
        .iter()
        .find(|x| x.child_count() == 0 && !x.kind().contains("comment"));
    if next.is_none_or(|x| x.start_position().row > row) {
        v.push(at(
            n,
            "UnnecessarySemicolon",
            "Semicolons as line endings can be removed safely".into(),
        ));
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

fn assignment_in_condition(n: Node, v: &mut Vec<Violation>) {
    if let Some(c) = condition(n)
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
        let t = text(val, src);
        if !seen.insert(t) {
            v.push(at(
                val,
                "DuplicateCaseStatement",
                format!("Duplicate case label {t}"),
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
    if l.kind() == "identifier" && r.kind() == "identifier" && text(l, src) == text(r, src) {
        v.push(at(
            n,
            "ComparisonWithSelf",
            format!("Comparing `{}` with itself", text(l, src)),
        ));
    }
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
        let src = "def m = [a: 1, 'a': 2, b: 3, 1: 'x', '1': 'y', \"${k}\": 4, \"${k}\": 5]\nswitch (x) {\n  case 1: f(); break\n  case 2: g(); break\n  case 1: h(); break\n}\n";
        assert_eq!(
            rules(src),
            vec![("DuplicateMapKey", 1), ("DuplicateCaseStatement", 5)]
        );
    }

    #[test]
    fn every_emitted_rule_is_registered() {
        let src = "import a.B\nimport a.B\ndef s = \"x\";\ntry { f() } catch (e) { }\nif (true) { }\nif (a = 1) { f() } else { }\ntry { } finally { }\nwhile (a) { }\nfor (i in xs) { }\ndef m = [a: 1, a: 2]\nswitch (x) { case 1: f(); break\n case 1: g() }\nif (x == x) { f() }\n";
        for v in lint(src).expect("parses") {
            assert!(
                RULES.iter().any(|r| r.name == v.rule),
                "{} missing from RULES",
                v.rule
            );
        }
    }

    #[test]
    fn unparseable_is_not_linted() {
        assert_eq!(lint("def x = {\n"), Err("parse error"));
    }
}
