//! Spock feature methods: block order, empty `then:` / `expect:` blocks, data table rows.
//!
//! The legal block sequences are Spock's own (`BlockParseInfo.getSuccessors` in
//! spock-core's compiler, and https://spockframework.org/spock/docs/2.4/spock_primer.html#_blocks);
//! Spock rejects anything else when the spec compiles. Data table rows are split the way
//! `WhereBlockRewriter.splitRow` does it: on every `|` and `||`, through parentheses, since
//! Groovy's AST has none.

use super::{Violation, at, has_interpolation};
use crate::text;
use tree_sitter::Node;

#[derive(Clone, Copy, PartialEq, Debug)]
enum B {
    Anonymous,
    Setup,
    Expect,
    When,
    Then,
    Cleanup,
    Where,
    Filter,
    And,
    Combined,
    End,
}

fn block(label: &str) -> Option<B> {
    Some(match label {
        "setup" | "given" => B::Setup,
        "expect" => B::Expect,
        "when" => B::When,
        "then" => B::Then,
        "cleanup" => B::Cleanup,
        "where" => B::Where,
        "filter" => B::Filter,
        "and" => B::And,
        "combined" => B::Combined,
        _ => return None,
    })
}

fn successors(b: B) -> &'static [B] {
    use B::*;
    match b {
        Anonymous => &[Setup, Expect, When, Cleanup, Where, End],
        Setup => &[And, Expect, When, Cleanup, Where, End],
        Expect => &[And, When, Cleanup, Where, End],
        When => &[And, Then],
        Then => &[And, Expect, When, Then, Cleanup, Where, End],
        Cleanup => &[And, Where, End],
        Where | Combined => &[And, Combined, Filter, End],
        Filter => &[And, End],
        And | End => &[],
    }
}

fn name(b: B) -> &'static str {
    use B::*;
    match b {
        Anonymous => "the start of the method",
        Setup => "given:",
        Expect => "expect:",
        When => "when:",
        Then => "then:",
        Cleanup => "cleanup:",
        Where => "where:",
        Filter => "filter:",
        And => "and:",
        Combined => "combined:",
        End => "the end of the method",
    }
}

/// Feature methods of a spec: every method with a block label, other than the fixture
/// methods, in a class whose superclass is `Specification` or a name ending in `Spec` or
/// `Specification` (a project's base spec). Labels in other classes are plain Groovy: Gradle's
/// JUnit integration tests use `when:` and `then:` as comments.
pub(super) fn class(src: &str, class: Node, v: &mut Vec<Violation>) {
    let spec = class
        .children(&mut class.walk())
        .find(|c| c.kind() == "superclass")
        .and_then(|s| s.named_child(0))
        .is_some_and(|t| {
            let t = text(t, src);
            let name = t
                .split('<')
                .next()
                .unwrap_or("")
                .rsplit('.')
                .next()
                .unwrap_or("");
            name.ends_with("Spec") || name.ends_with("Specification")
        });
    let Some(body) = class.child_by_field_name("body").filter(|_| spec) else {
        return;
    };
    for m in body
        .named_children(&mut body.walk())
        .filter(|m| m.kind() == "method_declaration")
    {
        let fixture = m.child_by_field_name("name").is_some_and(|n| {
            // `def "setup"()` is the fixture method too
            matches!(
                text(n, src).trim_matches(['"', '\'']),
                "setup" | "cleanup" | "setupSpec" | "cleanupSpec"
            )
        });
        if let Some(b) = m.child_by_field_name("body").filter(|_| !fixture) {
            feature(src, b, v);
        }
    }
}

fn feature(src: &str, body: Node, v: &mut Vec<Violation>) {
    let stmts: Vec<Node> = body
        .named_children(&mut body.walk())
        .filter(|s| !s.kind().contains("comment"))
        .collect();
    let labels: Vec<(Node, &str)> = stmts
        .iter()
        .filter(|s| s.kind() == "labeled_statement")
        .filter_map(|s| Some((*s, text(s.child_by_field_name("label")?, src))))
        .collect();
    if labels.is_empty()
        || labels.iter().any(|(s, _)| {
            // `a: b: x` is too unusual to model
            s.child_by_field_name("statement")
                .is_some_and(|x| x.kind() == "labeled_statement")
        })
    {
        return;
    }
    if let Some((s, l)) = labels.iter().find(|(_, l)| block(l).is_none()) {
        v.push(at(
            *s,
            "SpockBlockOrder",
            format!("`{l}:` is not a Spock block label, and Spock rejects other labels in a feature method; use given, when, then, expect, cleanup, where, and"),
        ));
        return;
    }
    // walk the blocks: `last` is the current real block, `content` counts its statements
    let (mut last, mut last_label, mut content) = (B::Anonymous, None, 0usize);
    let mut table = Table::default();
    for &s in &stmts {
        let mut stmt = s;
        if s.kind() == "labeled_statement" {
            let l = text(s.child_by_field_name("label").unwrap(), src);
            let b = block(l).unwrap();
            if !successors(last).contains(&b) {
                v.push(at(s, "SpockBlockOrder", order_message(l, last)));
                return;
            }
            if b != B::And && b != B::Combined {
                empty_check(last, last_label, content, v);
                (last, last_label, content) = (b, Some(s), 0);
            }
            if b == B::Combined || b == B::Where {
                table = Table::default();
            }
            match s.child_by_field_name("statement") {
                // a plain string after a label is its description
                Some(x) if is_description(x) => continue,
                Some(x) => stmt = x,
                None => continue,
            }
        }
        content += 1;
        if matches!(last, B::Where) {
            table.row(src, stmt, v);
        }
    }
    if !successors(last).contains(&B::End) {
        v.push(at(
            last_label.unwrap_or(body),
            "SpockBlockOrder",
            order_message("", last),
        ));
        return;
    }
    empty_check(last, last_label, content, v);
}

fn order_message(label: &str, last: B) -> String {
    let allowed: Vec<&str> = successors(last).iter().map(|b| name(*b)).collect();
    if label.is_empty() {
        format!(
            "The method ends after {}; Spock expects {} next",
            name(last),
            allowed.join(", ")
        )
    } else {
        format!(
            "`{label}:` can't follow {}; Spock allows {} there",
            name(last),
            allowed.join(", ")
        )
    }
}

fn is_description(x: Node) -> bool {
    x.kind() == "expression_statement"
        && x.named_child_count() == 1
        && x.named_child(0)
            .is_some_and(|e| e.kind() == "string_literal" && !has_interpolation(e))
}

fn empty_check(b: B, label: Option<Node>, content: usize, v: &mut Vec<Violation>) {
    if content == 0
        && matches!(b, B::Then | B::Expect)
        && let Some(l) = label
    {
        v.push(at(
            l,
            "SpockEmptyBlock",
            format!(
                "This {} block is empty, so the feature checks nothing here; add a condition or remove the block",
                name(b)
            ),
        ));
    }
}

/// Rows of the current data table: the cell count of the last row, if the table is going.
#[derive(Default)]
struct Table {
    cells: Option<usize>,
}

impl Table {
    fn row(&mut self, src: &str, s: Node, v: &mut Vec<Violation>) {
        let row = s
            .named_child(0)
            .filter(|e| s.kind() == "expression_statement" && is_or(*e, src));
        // anything else ends the table; so does a row the grammar may have joined across
        // lines (a line starting with `(` or `-` can read as a continuation)
        let Some(e) = row.filter(|_| s.start_position().row == s.end_position().row) else {
            self.cells = None;
            return;
        };
        let n = cells(e, src);
        if let Some(prev) = self.cells
            && prev != n
        {
            v.push(at(
                s,
                "SpockDataTableColumns",
                format!("This data table row has {n} columns but the row above has {prev}; Spock rejects the table"),
            ));
        }
        self.cells = Some(n);
    }
}

fn is_or(e: Node, src: &str) -> bool {
    e.kind() == "binary_expression"
        && e.child_by_field_name("operator")
            .is_some_and(|o| matches!(text(o, src), "|" | "||"))
}

/// Iterative: a row of n cells is a `|` tree n deep, too deep to recurse on.
fn cells(e: Node, src: &str) -> usize {
    let (mut n, mut todo) = (0, vec![e]);
    while let Some(mut e) = todo.pop() {
        while e.kind() == "parenthesized_expression"
            && let Some(x) = e.named_child(0)
        {
            e = x;
        }
        match (
            e.child_by_field_name("left"),
            e.child_by_field_name("right"),
        ) {
            (Some(l), Some(r)) if is_or(e, src) => todo.extend([l, r]),
            _ => n += 1,
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use crate::lint::lint;

    fn rules(body: &str) -> Vec<(&'static str, usize)> {
        let src = format!("class S extends Specification {{\n  def \"f\"() {{\n{body}\n  }}\n}}\n");
        lint(&src)
            .expect("parses")
            .into_iter()
            .filter(|v| v.rule.starts_with("Spock"))
            .map(|v| (v.rule, v.line))
            .collect()
    }

    #[test]
    fn legal_block_sequences() {
        for body in [
            "given: 'x'\ndef a = 1\nwhen:\na++\nthen:\na == 2\nand: 'more'\na > 0\nwhen:\na++\nthen:\na == 3\nthen:\na > 2\nexpect:\na\ncleanup:\na = 0\nwhere:\nx << [1]\nfilter:\nx > 0",
            "def a = 1\nexpect:\na == 1",
            "setup:\ndef a = 1\nexpect: a\nwhen: a++\nthen: a == 2",
            "expect:\nx\nwhere:\nx << [1]\ncombined:\ny << [2]",
            "when:\nf()\nand:\ng()\nthen:\nthrown(E)",
        ] {
            assert_eq!(rules(body), vec![], "{body}");
        }
    }

    #[test]
    fn illegal_block_sequences() {
        assert_eq!(
            rules("given:\ndef a = 1\nthen:\na == 1"),
            vec![("SpockBlockOrder", 5)]
        );
        assert_eq!(
            rules("when:\nf()\nwhen:\ng()\nthen:\nx"),
            vec![("SpockBlockOrder", 5)]
        );
        assert_eq!(
            rules("when:\nf()\nexpect:\nx"),
            vec![("SpockBlockOrder", 5)]
        );
        assert_eq!(
            rules("f()\nand:\ng()\nexpect:\nx"),
            vec![("SpockBlockOrder", 4)]
        );
        assert_eq!(rules("expect:\nx\ngiven:\ny"), vec![("SpockBlockOrder", 5)]);
        assert_eq!(
            rules("where:\nx << [1]\nexpect:\nx"),
            vec![("SpockBlockOrder", 5)]
        );
        // a `when:` the method ends after
        assert_eq!(rules("given:\nx\nwhen:\nf()"), vec![("SpockBlockOrder", 5)]);
        // a label Spock doesn't know
        assert_eq!(
            rules("expect:\nx\nouter:\nfor (i in xs) { }"),
            vec![("SpockBlockOrder", 5)]
        );
    }

    #[test]
    fn only_specs_and_feature_methods() {
        // not a spec: Gradle's JUnit tests use labels as comments
        let src =
            "class T extends AbstractIntegrationTest {\n  void t() {\n    then:\n    x\n  }\n}\n";
        assert_eq!(lint(src).unwrap(), vec![]);
        // fixture methods ignore labels; a base spec counts as a spec
        let src = "class T extends BaseSpec {\n  def setup() {\n    then:\n    x\n  }\n  def \"cleanup\"() {\n    when:\n    x\n  }\n  def f() {\n    when:\n    x\n  }\n}\n";
        let v: Vec<_> = lint(src).unwrap().into_iter().map(|v| v.line).collect();
        assert_eq!(v, vec![11]);
    }

    #[test]
    fn empty_blocks() {
        assert_eq!(
            rules(
                "when:\nf()\nthen: 'nothing'\nwhen:\ng()\nthen:\nx\nexpect: 'nothing'\nand: 'still nothing'"
            ),
            vec![("SpockEmptyBlock", 5), ("SpockEmptyBlock", 10)]
        );
        assert_eq!(rules("when:\nf()\nthen: 'desc'\nand:\nx == 1"), vec![]);
        // a GString is a condition, not a description
        assert_eq!(rules("expect: \"${x}\""), vec![]);
    }

    #[test]
    fn data_table_columns() {
        let body = "expect:\nx\nwhere:\na | b || c\n1 | 2 || 3\n4 | 5\n6 | (7 | 8) || 9\n__\nd | e\n1 | 2\nand:\n3 | 4\ncombined:\nf | g | h\n1 | 2 | 3";
        assert_eq!(
            rules(body),
            vec![("SpockDataTableColumns", 8), ("SpockDataTableColumns", 9)]
        );
        // a row 50,000 cells wide is a `|` tree that deep: counted without a stack overflow
        let row = vec!["a"; 50_000].join(" | ");
        assert_eq!(
            rules(&format!("expect:\nx\nwhere:\n{row}\n{row} | b")),
            vec![("SpockDataTableColumns", 7)]
        );
    }
}
