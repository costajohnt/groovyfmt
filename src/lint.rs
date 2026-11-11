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
}

/// Every rule, named after its CodeNarc equivalent so existing configs map across.
pub const RULES: &[Rule] = &[
    Rule {
        name: "UnusedImport",
        default: true,
        description: "import whose name (or alias) never appears in code or as a word in a comment",
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
        description: "`=` where `==` was probably meant, in an if condition (`while (x = next())` is a common idiom, so loops are exempt, and so is an assignment in extra parentheses)",
    },
    Rule {
        name: "ComparisonWithSelf",
        default: false,
        description: "a variable compared with itself (off by default: Spock equality checks do this on purpose; GORM `where { title == title }` is exempt)",
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
    // Declarative Jenkins pipelines (src/lint/jenkins.rs)
    Rule {
        name: "JenkinsMissingSection",
        default: true,
        description: "pipeline without agent or stages, stages without a stage, a stage with none of steps, stages, parallel, matrix",
    },
    Rule {
        name: "JenkinsConflictingSections",
        default: true,
        description: "a section twice in one block, or a stage with more than one of steps, stages, parallel, matrix",
    },
    Rule {
        name: "JenkinsUnknownSection",
        default: true,
        description: "a section that doesn't belong where it is in pipeline, stage, matrix, post or stages",
    },
    Rule {
        name: "JenkinsStatementOutsideSection",
        default: true,
        description: "Groovy code directly in pipeline, stages, stage, matrix, post or when instead of in steps",
    },
    Rule {
        name: "JenkinsScriptRequired",
        default: true,
        description: "if, for, while, try, switch, def or an assignment directly in steps without script { }",
    },
    Rule {
        name: "JenkinsUnknownWhenCondition",
        default: true,
        description: "a when condition that isn't built in",
    },
    Rule {
        name: "JenkinsMisplacedOption",
        default: true,
        description: "parameters or triggers in options, or a pipeline-wide option in a stage's options",
    },
    Rule {
        name: "JenkinsDuplicateStageName",
        default: true,
        description: "two stages with the same name in one stages or parallel block",
    },
    Rule {
        name: "JenkinsStageName",
        default: true,
        description: "a stage with no name, or a name that isn't a plain string",
    },
    Rule {
        name: "JenkinsAgentForm",
        default: true,
        description: "agent with no value, an empty or multi-type block, or a bare type other than any and none",
    },
    // Gradle build scripts (src/lint/gradle.rs)
    Rule {
        name: "GradleRemovedConfiguration",
        default: true,
        description: "compile, runtime, testCompile or testRuntime dependencies (removed in Gradle 7.0)",
    },
    Rule {
        name: "GradleJcenter",
        default: true,
        description: "the jcenter() repository (removed in Gradle 9.0)",
    },
    Rule {
        name: "GradleLeftShiftTask",
        default: true,
        description: "task name << { } (removed in Gradle 5.0)",
    },
    Rule {
        name: "GradleDynamicVersion",
        default: false,
        description: "a + or latest.* dependency version (off by default: a policy choice)",
    },
    // Spock specifications (src/lint/spock.rs)
    Rule {
        name: "SpockBlockOrder",
        default: true,
        description: "feature method blocks in an order Spock rejects, or a label Spock doesn't know",
    },
    Rule {
        name: "SpockEmptyBlock",
        default: false,
        description: "an empty then: or expect: block (off by default: Spock allows it, and its own tests use empty blocks on purpose)",
    },
    Rule {
        name: "SpockDataTableColumns",
        default: true,
        description: "a data table row with a different number of columns than the row above",
    },
];

mod gradle;
mod jenkins;
mod spock;

#[derive(Debug, PartialEq)]
pub struct Violation {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based
    pub rule: &'static str,
    pub message: String,
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
            "class_declaration" => spock::class(src, n, &mut v),
            _ => {}
        }
    }
    // tree-sitter columns count bytes; report characters, as editors and GitHub do
    let lines: Vec<&str> = src.split('\n').collect();
    for x in &mut v {
        if let Some(l) = lines.get(x.line - 1) {
            x.col = l
                .get(..x.col - 1)
                .map_or(x.col, |prefix| prefix.chars().count() + 1);
        }
    }
    v.sort_by_key(|x| (x.line, x.col));
    Ok(v)
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
        if !symbol.is_empty() && !used.contains(symbol) {
            v.push(at(
                d,
                "UnusedImport",
                format!("The [{name}] import is never referenced"),
            ));
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
/// (`foo();` then `{ -> x }.call()`): either way removing it would change the program,
/// here by making the closure an argument of the statement before.
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
    if next.is_none_or(|x| x.start_position().row > row && x.kind() != "{") {
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
        for (name, src) in [
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
            (v[0].rule, v[0].line, v[0].col),
            ("UnnecessarySemicolon", 1, 12)
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
    fn unparseable_is_not_linted() {
        assert_eq!(lint("def x = {\n"), Err("parse error"));
    }
}
