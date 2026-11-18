//! Spock feature methods: block order, empty `then:` / `expect:` blocks, data table rows,
//! conditions Spock never asserts.
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

/// The simple name of `class`'s superclass: `Base` for `extends com.x.Base<T>`.
pub(super) fn superclass<'a>(class: Node, src: &'a str) -> Option<&'a str> {
    let t = class
        .children(&mut class.walk())
        .find(|c| c.kind() == "superclass")?
        .named_child(0)?;
    let name = text(t, src).split('<').next()?.rsplit('.').next()?.trim();
    (!name.is_empty()).then_some(name)
}

/// Classes by simple name, with each declaration's superclass (see `superclass`).
pub(super) type Classes = std::collections::HashMap<Box<str>, Vec<Option<Box<str>>>>;

/// Adds the classes declared under `root` to `classes`.
pub(super) fn classes(root: Node, src: &str, classes: &mut Classes) {
    crate::preorder(root, |n| {
        if n.kind() == "class_declaration"
            && let Some(name) = n.child_by_field_name("name")
        {
            let sup = superclass(n, src).map(Box::from);
            classes.entry(text(name, src).into()).or_default().push(sup);
        }
        true
    });
}

/// Whether the class `name` is a spec: named like one, or every declaration of it in
/// `classes` extends one (`Some(true)`) or ends at a class extending nothing (`Some(false)`).
/// A name `classes` doesn't declare is asked of `outside`; declarations that disagree, a
/// cycle, or a chain over 32 classes long, are `None`.
pub(super) fn spec(
    classes: &Classes,
    name: &str,
    outside: &dyn Fn(&str) -> Option<bool>,
) -> Option<bool> {
    // each name's verdict, worked out once: many declarations of one name at each level
    // would otherwise be walked once per path to them, exponentially. `None` while it is
    // being worked out, so a cycle is unknown.
    type Memo<'a> = std::collections::HashMap<&'a str, Option<Option<bool>>>;
    fn go<'a>(
        classes: &'a Classes,
        name: &'a str,
        outside: &dyn Fn(&str) -> Option<bool>,
        memo: &mut Memo<'a>,
        depth: usize,
    ) -> Option<bool> {
        if spec_named(name) {
            return Some(true);
        }
        let Some(decls) = classes.get(name) else {
            return outside(name);
        };
        if let Some(known) = memo.get(name) {
            return known.flatten();
        }
        // ponytail: past this depth a chain is unknown, so the recursion can't overflow
        if depth > 32 {
            return None;
        }
        memo.insert(name, None);
        let mut verdicts = decls.iter().map(|sup| match sup {
            None => Some(false),
            Some(s) => go(classes, s, outside, memo, depth + 1),
        });
        let verdict = verdicts
            .next()
            .flatten()
            .filter(|&first| verdicts.all(|v| v == Some(first)));
        memo.insert(name, Some(verdict));
        verdict
    }
    go(classes, name, outside, &mut Memo::new(), 0)
}

/// A name Spock's own bases and a project's base specs end with.
pub(super) fn spec_named(name: &str) -> bool {
    name.ends_with("Spec") || name.ends_with("Specification")
}

/// Feature methods of a spec: every method with a block label, other than the fixture
/// methods, in a class that extends a spec. That is a superclass named like one (`Specification`,
/// `GebSpec`, a project's `BaseSpec`); else one whose chain, through this file's classes
/// (`local`) and then the project's (`base`), reaches a spec; else, when neither can tell
/// (no project, or a base from a library), a file importing from `spock.`. Labels in
/// other classes are plain Groovy: Gradle's JUnit integration tests use `when:` and `then:` as
/// comments.
pub(super) fn class(
    src: &str,
    class: Node,
    local: &std::cell::OnceCell<Classes>,
    base: &dyn Fn(&str) -> Option<bool>,
    v: &mut Vec<Violation>,
) {
    let Some(sup) = superclass(class, src) else {
        return;
    };
    let Some(body) = class.child_by_field_name("body") else {
        return;
    };
    let labeled = |m: Node| {
        m.child_by_field_name("body").is_some_and(|b| {
            b.named_children(&mut b.walk())
                .any(|s| s.kind() == "labeled_statement")
        })
    };
    // asked only for a class with labels, so a project is read only when it may matter
    let spec = spec_named(sup)
        || body
            .named_children(&mut body.walk())
            .any(|m| m.kind() == "method_declaration" && labeled(m))
            && {
                let local = local.get_or_init(|| {
                    let root = std::iter::successors(Some(class), |n| n.parent()).last();
                    let mut c = Classes::new();
                    classes(root.unwrap_or(class), src, &mut c);
                    c
                });
                spec(local, sup, base).unwrap_or_else(|| imports_spock(src))
            };
    if !spec {
        return;
    }
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

/// Whether `src` imports from Spock (`import spock.lang.Unroll`): a test class extending a
/// base no file at hand declares is taken for a spec only then.
fn imports_spock(src: &str) -> bool {
    src.lines().any(|l| {
        let l = l.trim_start();
        let l = l.strip_prefix("import").unwrap_or("");
        let l = l.trim_start();
        let l = l.strip_prefix("static").map_or(l, str::trim_start);
        l.starts_with("spock.")
    })
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
        if matches!(last, B::Then | B::Expect) {
            unasserted(src, stmt, None, false, v);
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

/// Closures whose result their method throws away, so a condition in them checks nothing.
/// Others are left alone: `every { }` and `find { }` return their closure's value, which
/// Spock asserts at the top level, and Spock reads `with { }`, `verifyAll { }`, `verifyEach`,
/// `interaction { }` and argument constraints as conditions of their own.
const DISCARDING: &[&str] = &["each", "eachWithIndex", "reverseEach", "forEach", "times"];

/// The name of the method a closure is passed to, and whether it is called on a receiver.
fn closure_call<'a>(n: Node<'a>, src: &'a str) -> (Option<Node<'a>>, Option<&'a str>) {
    let call = n
        .parent()
        .filter(|p| p.kind() == "argument_list")
        .map_or(n.parent(), |a| a.parent())
        .filter(|c| c.kind() == "method_invocation");
    let f = call.and_then(|c| c.child_by_field_name("function"));
    let field = f.and_then(|f| f.child_by_field_name("field"));
    (field, field.or(f).map(|f| text(f, src)))
}

/// Whether the closure `n` goes to a method in DISCARDING, which drops its value.
pub(super) fn discarded(n: Node, src: &str) -> bool {
    closure_call(n, src)
        .1
        .is_some_and(|m| DISCARDING.contains(&m))
}

/// SpockUnassertedCondition: in a then: or expect: block Spock asserts only top-level
/// expression statements. A comparison nested in `if`, a loop, `switch`, `try` or a
/// `.each { }` closure is evaluated and dropped (checked with Spock 2.4). `inside` names
/// the construct around `n` once it is no longer at the top level (for a closure, its
/// method's name). `with` is set in a `with { }` or `verifyAll { }` closure Spock asserts:
/// there it asserts what an `if` or a loop holds too, but not what a `.each { }` closure
/// holds (`x.with { xs.each { it == 2 } }`). In a closure that drops its conditions, Groovy's
/// `it.with { }` drops them too, unlike Spock's own `with(it) { }` (all checked with Spock 2.4).
fn unasserted<'a>(
    src: &'a str,
    n: Node,
    inside: Option<&'a str>,
    with: bool,
    v: &mut Vec<Violation>,
) {
    let (inside, with) = match n.kind() {
        "if_statement" if !with => (Some("an if"), false),
        "for_statement" | "for_in_statement" | "while_statement" | "do_while_statement"
            if !with =>
        {
            (Some("a loop"), false)
        }
        "switch_case" | "switch_default" if !with => (Some("a switch"), false),
        "try_statement" if !with => (Some("a try"), false),
        "synchronized_statement" if !with => (Some("a synchronized block"), false),
        // anything a closure's method may run, or not, as it likes; and classes and lambdas
        "closure" => {
            let (field, m) = closure_call(n, src);
            match m {
                Some(m) if DISCARDING.contains(&m) => (Some(m), false),
                // `x.with { }`, Groovy's, in a closure that drops its conditions
                Some("with") if inside.is_some() && field.is_some() => (inside, false),
                Some("with" | "verifyAll") if inside.is_none() => (None, true),
                _ => return,
            }
        }
        "lambda_expression" | "class_body" | "switch_arrow_case" => return,
        _ => (inside, with),
    };
    if let Some(ctx) = inside
        && (n.kind() == "expression_statement"
            && n.named_child(0)
                .is_some_and(|e| is_condition(e, src, reads_check(n, src)))
            || n.kind() == "command_chain" && is_predicate(n, src))
    {
        let ctx = if DISCARDING.contains(&ctx) {
            format!("a .{ctx} {{ }} closure")
        } else {
            ctx.to_string()
        };
        v.push(at(
            n,
            "SpockUnassertedCondition",
            format!("This condition is inside {ctx} in a Spock then: or expect: block, so Spock never asserts it; add `assert`, or check it with verifyAll {{ }} or verifyEach"),
        ));
        return;
    }
    for c in n.named_children(&mut n.walk()) {
        unasserted(src, c, inside, with, v);
    }
}

/// A comparison, an instanceof or `in` test, a predicate call (`xs.contains(x)`, see
/// PREDICATES), or `!`, `&&` or `||` over them: an expression
/// statement made only of checks, with nothing it could be run for (`x != null && x.close()`
/// is not one).
fn is_condition(e: Node, src: &str, reads: bool) -> bool {
    let op = || e.child_by_field_name("operator").map(|o| text(o, src));
    match e.kind() {
        "parenthesized_expression" => e
            .named_child(0)
            .is_some_and(|x| is_condition(x, src, reads)),
        "instanceof_expression" | "identity_expression" | "membership_expression" => true,
        "unary_expression" => {
            op() == Some("!")
                && e.child_by_field_name("operand")
                    .is_some_and(|x| is_condition(x, src, reads))
        }
        "binary_expression" => match op() {
            Some("==" | "!=" | "<" | "<=" | ">" | ">=") => true,
            Some("&&" | "||") => ["left", "right"].iter().all(|f| {
                e.child_by_field_name(f)
                    .is_some_and(|x| is_condition(x, src, reads))
            }),
            _ => false,
        },
        "method_invocation" => is_predicate(e, src),
        "field_access" | "safe_navigation_expression" => reads && is_property_read(e),
        _ => false,
    }
}

/// Whether a property read in the statement `n` is a check: not when a `try` around it may
/// be waiting for the read to throw (`try { bean.missing; assert false } catch (...)`, a Geb
/// `page.requiredDiv`), nor when more code follows on its line, which the grammar may have
/// split off a command call (`operation.child applyPlugin(p)`).
fn reads_check(n: Node, src: &str) -> bool {
    let rest = src[n.end_byte()..].trim_start_matches([' ', '\t']);
    let line_ends = rest.is_empty() || rest.starts_with(['\n', '\r', ';', '}', '/']);
    line_ends
        && !std::iter::successors(n.parent(), |p| p.parent()).any(|p| p.kind() == "try_statement")
}

/// A property read on a plain path, `x.done`, `it?.ok`, `xs[0].terminal`, `this.valid`:
/// no call anywhere in it, so evaluating it does nothing but answer (#212).
fn is_property_read(mut e: Node) -> bool {
    loop {
        e = match e.kind() {
            "field_access" | "safe_navigation_expression" | "parenthesized_expression" => {
                match e.child_by_field_name("object").or_else(|| e.named_child(0)) {
                    Some(o) => o,
                    None => return false,
                }
            }
            "subscript_expression" => {
                let plain = e.child_by_field_name("index").is_some_and(|i| {
                    matches!(i.kind(), "number_literal" | "identifier")
                        || i.kind() == "string_literal" && !has_interpolation(i)
                });
                match e.child_by_field_name("object") {
                    Some(o) if plain => o,
                    _ => return false,
                }
            }
            "identifier" | "this" => return true,
            _ => return false,
        };
    }
}

/// Calls that only answer a question, so a nested one is as unasserted as a nested
/// comparison: well-known side-effect-free JDK, Groovy and Grails names only. Not a bare
/// `contains(x)`, which may be a spec helper that asserts, nor a call in an assertion chain,
/// `assertThat(s).contains(x)`, which checks by throwing.
const PREDICATES: &[&str] = &[
    "contains",
    "containsAll",
    "containsKey",
    "containsValue",
    "endsWith",
    "equals",
    "equalsIgnoreCase",
    "hasErrors",
    "hasNext",
    "isAssignableFrom",
    "isClosed",
    "isEmpty",
    "isPresent",
    "matches",
    "startsWith",
];

/// A call named in PREDICATES on a receiver, with no closure, and no assertion call
/// (`assertThat`, `expect`, `verify`, `that`, ...) anywhere in the receiver chain. `e` may
/// be a command chain of one argument: `it.startsWith "a"`.
fn is_predicate<'a>(e: Node<'a>, src: &'a str) -> bool {
    // `x.m` or `x?.m`
    let member = |f: Node<'a>| {
        f.child_by_field_name("field")
            .or_else(|| f.child_by_field_name("property"))
    };
    let name = |call: Node<'a>| {
        call.child_by_field_name("function")
            .or_else(|| call.child_by_field_name("receiver"))
            .map(|f| (text(member(f).unwrap_or(f), src), f))
    };
    let Some((m, f)) =
        name(e).filter(|(_, f)| matches!(f.kind(), "field_access" | "safe_navigation_expression"))
    else {
        return false;
    };
    if !PREDICATES.contains(&m)
        || e.child_by_field_name("closure").is_some()
        || e.kind() == "command_chain"
            && (e.named_child_count() != 2
                || e.child_by_field_name("argument")
                    .is_none_or(|a| a.kind() == "closure"))
    {
        return false;
    }
    let mut r = f.child_by_field_name("object");
    while let Some(n) = r {
        r = match n.kind() {
            "method_invocation" => {
                let Some((m, f)) = name(n) else { break };
                if m == "that"
                    || ["assert", "assume", "check", "expect", "verify"]
                        .iter()
                        .any(|p| m.starts_with(p))
                {
                    return false;
                }
                f.child_by_field_name("object")
            }
            "field_access" | "safe_navigation_expression" => n.child_by_field_name("object"),
            "parenthesized_expression" => n.named_child(0),
            _ => None,
        };
    }
    true
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
    fn specs_by_their_base_class() {
        let unasserted = |src: &str| -> Vec<usize> {
            lint(src)
                .unwrap()
                .into_iter()
                .filter(|v| v.rule == "SpockUnassertedCondition")
                .map(|v| v.line)
                .collect()
        };
        let feature = "  def \"f\"() {\n    expect:\n    if (c) { x == 1 }\n  }\n";
        // a base in the same file, directly or through a chain, generic or qualified
        for decls in [
            "abstract class Base extends Specification { }\n",
            "class Mid<T> extends spock.lang.Specification { }\nclass Base extends Mid<String> { }\n",
            "class A extends Specification { }\nclass B extends A { }\nclass C extends B { }\nclass Base extends C { }\n",
        ] {
            let src = format!("{decls}class T extends Base {{\n{feature}}}\n");
            assert_eq!(unasserted(&src).len(), 1, "{src}");
        }
        // a base this file declares that isn't a spec, also when the file imports Spock or
        // the class is named like a spec
        for src in [
            format!("class Base {{ }}\nclass T extends Base {{\n{feature}}}\n"),
            format!(
                "import spock.lang.Unroll\nclass Base {{ }}\nclass Base2 extends Base {{ }}\nclass FooSpec extends Base2 {{\n{feature}}}\n"
            ),
            // two declarations that disagree, and a cycle: unknown, and no Spock import
            format!(
                "class Base extends Specification {{ }}\nclass Base {{ }}\nclass T extends Base {{\n{feature}}}\n"
            ),
            format!(
                "class A extends B {{ }}\nclass B extends A {{ }}\nclass T extends A {{\n{feature}}}\n"
            ),
        ] {
            assert_eq!(unasserted(&src), Vec::<usize>::new(), "{src}");
        }
        // a base declared nowhere: a spec only in a file importing from Spock
        let src = format!("class T extends AbstractIntegrationTest {{\n{feature}}}\n");
        assert_eq!(unasserted(&src), Vec::<usize>::new());
        for import in [
            "import spock.lang.Unroll",
            "import static spock.util.matcher.HamcrestSupport.that",
        ] {
            let src = format!("{import}\n{src}");
            assert_eq!(unasserted(&src), vec![5], "{src}");
        }
        // a base in another file: as the project tells (`Projects::spec_base`)
        let src = format!("class T extends Base {{\n{feature}}}\n");
        for (verdict, want) in [(Some(true), vec![4]), (Some(false), vec![]), (None, vec![])] {
            let vs = crate::lint::lint_with(std::path::Path::new(""), &src, &|n| {
                assert_eq!(n, "Base");
                verdict
            });
            let got: Vec<_> = vs.unwrap().into_iter().map(|v| v.line).collect();
            assert_eq!(got, want, "{verdict:?}");
        }
    }

    #[test]
    fn base_class_resolution_is_linear() {
        use super::{Classes, spec};
        // `width` declarations of each of 30 names, each extending every one of the next
        // name's: walked per path, that is width^30 visits. Then a cycle beside it.
        let run = |width: usize| {
            let mut classes = Classes::new();
            for level in 0..30 {
                let sup: Box<str> = if level == 29 {
                    "Specification".into()
                } else {
                    format!("L{}", level + 1).into()
                };
                classes.insert(format!("L{level}").into(), vec![Some(sup); width]);
            }
            classes.insert("A".into(), vec![Some("B".into()); width]);
            classes.insert("B".into(), vec![Some("A".into()); width]);
            assert_eq!(spec(&classes, "L0", &|_| None), Some(true));
            assert_eq!(spec(&classes, "A", &|_| None), None);
        };
        crate::scaling::assert_scales("base class resolution", run, 64, 4, 8);
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

    fn unasserted(body: &str) -> Vec<usize> {
        rules(body)
            .into_iter()
            .filter(|(r, _)| *r == "SpockUnassertedCondition")
            .map(|(_, l)| l)
            .collect()
    }

    /// Each nesting Spock 2.4 was seen to drop: a feature whose only check sits there passes
    /// whatever the values.
    #[test]
    fn unasserted_conditions() {
        // (body, the reported line: the body starts on line 3)
        for (body, line) in [
            ("when:\nf()\nthen:\nif (a) {\nx == 2\n}", 7),
            ("when:\nf()\nthen:\nif (a) {\ntrue\n} else {\nx != 2\n}", 9),
            ("when:\nf()\nthen:\nif (a) {\n} else if (b) {\nx < 2\n}", 8),
            ("when:\nf()\nthen:\nif (a)\nx >= 2", 7),
            (
                "when:\nf()\nthen:\nfor (int i = 0; i < n; i++) {\nxs[i] == 2\n}",
                7,
            ),
            (
                "when:\nf()\nthen:\nfor (x in xs) {\nx instanceof String\n}",
                7,
            ),
            ("when:\nf()\nthen:\nwhile (n-- > 0) {\nn == 5\n}", 7),
            (
                "when:\nf()\nthen:\nswitch (x) {\ncase 1:\nx == 2\nbreak\n}",
                8,
            ),
            ("when:\nf()\nthen:\ntry {\nx == 2\n} finally {\n}", 7),
            ("when:\nf()\nthen:\nsynchronized (this) {\nx === y\n}", 7),
            ("when:\nf()\nthen:\nxs.each {\nit == 2\n}", 7),
            (
                "when:\nf()\nthen:\nxs.eachWithIndex { v, i ->\nv == 2\n}",
                7,
            ),
            ("when:\nf()\nthen:\nxs.forEach({\nit in ys\n})", 7),
            ("when:\nf()\nthen:\nn.times {\n!(it == 5)\n}", 7),
            ("when:\nf()\nthen:\nxs.each {\nif (it) {\nit == 2\n}\n}", 8),
            (
                "when:\nf()\nthen:\nif (a) {\nxs.each {\nit == 2 && it > 0\n}\n}",
                8,
            ),
            ("expect:\nif (a) {\n1 == 2\n}", 5),
            ("when:\nf()\nthen:\nx == 1\nand:\nif (a) {\n(x == 2)\n}", 9),
            // #207: an and: after a then: holding a command chain
            (
                "when:\nf()\nthen:\ndef t = x\nt M.dependsOn('a')\n\nand:\nxs.each { c ->\nc.a == 1\n}",
                11,
            ),
            // #207: predicate calls
            ("when:\nf()\nthen:\nif (a) {\nxs.contains(5)\n}", 7),
            ("when:\nf()\nthen:\nxs.each {\n!it.isEmpty()\n}", 7),
            (
                "when:\nf()\nthen:\nif (a) {\n} else {\nm?.containsKey('k')\n}",
                8,
            ),
            (
                "when:\nf()\nthen:\nfor (x in xs) {\nx.startsWith('a') && x.endsWith('z')\n}",
                7,
            ),
            (
                "when:\nf()\nthen:\nswitch (x) {\ncase 1:\nx.text.matches(/a+/)\n}",
                8,
            ),
            (
                "when:\nf()\nthen:\ntry {\nArrays.equals(a, b)\n} catch (e) {\n}",
                7,
            ),
            ("when:\nf()\nthen:\nif (a) {\n!errors.hasErrors(true)\n}", 7),
            (
                "when:\nf()\nthen:\nn.times {\n(x as String).equalsIgnoreCase(cells[1].text()[0])\n}",
                7,
            ),
            (
                "expect:\nif (a) {\n$('body').text().contains('x') || o.isPresent()\n}",
                5,
            ),
            // #218: a .each { } in a with { } or verifyAll { }, Groovy's `it.with { }` in a
            // .each { } or an if, and a predicate called without parentheses
            ("when:\nf()\nthen:\nx.with {\nxs.each {\nit == 5\n}\n}", 8),
            ("when:\nf()\nthen:\nwith(x) {\nxs.each {\nit == 5\n}\n}", 8),
            (
                "when:\nf()\nthen:\nverifyAll(x) {\nif (c) {\nxs.each { it == 5 }\n}\n}",
                8,
            ),
            ("when:\nf()\nthen:\nxs.each {\nit.with {\na == 2\n}\n}", 8),
            (
                "when:\nf()\nthen:\nxs.each {\nit.with {\nif (b) {\na == 2\n}\n}\n}",
                9,
            ),
            ("when:\nf()\nthen:\nif (c) {\nx.with {\na == 2\n}\n}", 8),
            ("when:\nf()\nthen:\nxs.each {\nit.startsWith \"y\"\n}", 7),
            (
                "expect:\n[a: [1]].with {\na.each { b -> b.with { c == 2 } }\n}",
                5,
            ),
            // #212: a property read on a plain path, alone, under `!` or `&&`; isClosed()
            ("when:\nf()\nthen:\n[1].each {\nsnapshots[0].terminal\n}", 7),
            ("when:\nf()\nthen:\n[1].each {\n!snapshots[1].initial\n}", 7),
            ("when:\nf()\nthen:\nif (a) {\nx.ok\n}", 7),
            ("when:\nf()\nthen:\nfor (e in xs) {\nit?.a && e.b.c\n}", 7),
            ("when:\nf()\nthen:\nxs.each {\nthis.valid\n}", 7),
            ("when:\nf()\nthen:\nxs.each {\nm['k'].done\n}", 7),
            ("when:\nf()\nthen:\nxs.each {\n!cursor.isClosed()\n}", 7),
        ] {
            assert_eq!(unasserted(body), vec![line], "{body}");
        }
    }

    /// What Spock 2.4 does assert, or what isn't a check: never reported.
    #[test]
    fn asserted_or_not_conditions() {
        for body in [
            // top level
            "when:\nf()\nthen:\nx == 2\n!(x < 1)\nx instanceof String",
            // with, verifyAll and verifyEach assert what's nested in them, even in an if
            "when:\nf()\nthen:\nwith(x) {\nif (c) {\na == 2\n}\n}",
            "when:\nf()\nthen:\nverifyAll {\nif (c) {\nx == 2\n}\n}",
            "when:\nf()\nthen:\nverifyAll(x) {\na == 2\n}",
            "when:\nf()\nthen:\nverifyEach(xs) {\nif (c) {\nit == 2\n}\n}",
            "when:\nf()\nthen:\nif (c) {\nwith(x) {\na == 2\n}\n}",
            "when:\nf()\nthen:\nxs.each {\nverifyAll {\nit == 2\n}\n}",
            // #218: Groovy's `x.with { }` at the top level too, even in an if, a loop or a
            // nested with; and Spock's own with and verifyAll in a .each { }
            "when:\nf()\nthen:\nx.with {\na == 2\nif (c) {\na == 2\n}\nfor (i in a) {\ni == 5\n}\ny.with {\nb == 1\n}\n}",
            "when:\nf()\nthen:\nwith(x) {\nif (c) {\na == 2\n}\n}\nxs.each {\nwith(it) {\na == 2\n}\nverifyAll(it) {\na == 2\n}\n}",
            // a command-call predicate at the top level, or not a predicate
            "when:\nf()\nthen:\nit.startsWith \"y\"\nxs.each {\nit.add \"y\"\nit.contains { it }\nstartsWith \"y\"\n}",
            // interactions, argument constraints, interaction blocks, response closures
            "when:\nf()\nthen:\n1 * m.foo({ it == 2 })\ninteraction {\n1 * m.foo({ it == 2 })\n}",
            "when:\nf()\nthen:\nif (c) {\n1 * m.foo(2)\n}\n1 * m.foo(_) >> { args -> args[0] == 2 }",
            // closures whose value the call returns, and are asserted at the top level
            "when:\nf()\nthen:\nxs.every { it == 2 }\nxs.find { it == 2 }\nxs.any {\nif (c) {\nit == 2\n}\n}",
            // an explicit assert, a thrown() check, a call, a conditional call
            "when:\nf()\nthen:\nif (c) {\nassert x == 2\nthrown(E)\nx.size()\nx != null && x.close()\ny = x == 2\n}",
            // calls run for what they do, or that may assert themselves
            "when:\nf()\nthen:\nxs.each {\nlist.add(it)\nmap.put(it, 1)\nset.remove(it)\nit.contains(1) && it.close()\n}",
            "when:\nf()\nthen:\nif (c) {\ncontains(x)\nhasBaseEnvironment(env)\nx.isValid()\nx.contains { it }\n}",
            "when:\nf()\nthen:\nif (c) {\nassertThat(r).output().contains('x')\nassertAbout(t).that(r).isEmpty()\nexpect(s).startsWith('a')\nverify(m).equals(x)\n}",
            // predicates Spock asserts: top level, with, verifyAll, every, an argument constraint
            "when:\nf()\nthen:\nxs.contains(1)\n!m.isEmpty()\nwith(xs) {\nif (c) {\nit.contains(1)\n}\n}\nverifyAll {\nif (c) {\nxs.contains(1)\n}\n}\nxs.every { it.startsWith('a') }\n1 * m.foo({ it.contains('a') })",
            // #212: property reads Spock asserts, or with a call in the path, a computed
            // index, an assignment, or a bare name
            "when:\nf()\nthen:\nx.ok\n!xs[0].ok\nxs.every { it.ok }\nwith(x) {\nif (c) {\nx.ok\n}\n}\nxs.find { it.ok }",
            "when:\nf()\nthen:\nxs.each {\nf().ok\nit.next().ok\nxs[i + 1].ok\nm[\"$k\"].ok\nit.ok = true\nok\nxs[0]\n}",
            // #212: a read a try waits on to throw, and one the grammar split off a command call
            "expect:\ntry {\nbean.notBoolean\nassert false\n} catch (MissingPropertyException e) {\n}\nxs.each {\ntry { page.requiredDiv } catch (e) { }\n}",
            "when:\nf()\nthen:\nxs.each { op ->\nop.child applyPlugin(p1)\n}",
            // not then: or expect:
            "given:\nif (c) {\nx == 2\n}\nexpect:\ntrue\ncleanup:\nif (c) {\nx == 2\n}",
            "when:\nif (c) {\nx == 2\n}\nthen:\ntrue\nwhere:\nx << [1]",
            // a lambda or an anonymous class body is code of its own
            "when:\nf()\nthen:\nif (c) {\nxs.forEach(x -> x == 2)\n}",
        ] {
            assert_eq!(unasserted(body), Vec::<usize>::new(), "{body}");
        }
    }

    /// `@Verify` / `@VerifyAll` helpers and other methods without blocks are not features.
    #[test]
    fn verify_helpers_are_not_reported() {
        let src = "class S extends Specification {\n  @Verify\n  void check(x) {\n    if (c) {\n      x == 2\n    }\n    [x].each { it == 2 }\n  }\n  @VerifyAll\n  void all(x) {\n    if (c) { x == 2 }\n  }\n}\n";
        assert_eq!(lint(src).unwrap(), vec![]);
        // and a class that is not a spec
        let src = "class T {\n  void t() {\n    when:\n    f()\n    then:\n    if (c) {\n      x == 2\n    }\n  }\n}\n";
        assert_eq!(lint(src).unwrap(), vec![]);
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
